use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
};

use tokio::sync::{mpsc, watch};

use super::{
    BoundaryClaim, RuntimeCommand, RuntimeInputs, RuntimeRequestError, Shared, TestDispatchResult,
};
use crate::{
    app_log,
    config::{ActiveConfig, Device, SendAction},
    event::Event,
    hid::{HidBackend, HidError, HidInventory, HidRefreshState},
};

/// The sole owner of mutable HID state and event execution. After construction,
/// it is moved to the dedicated thread; async is used only to wait for work.
pub(super) struct AutomationWorker {
    shared: Arc<Shared>,
    inputs: RuntimeInputs,
    commands: mpsc::Receiver<RuntimeCommand>,
    shutdown: watch::Receiver<bool>,
    backend: Box<dyn HidBackend>,
}

impl AutomationWorker {
    pub(super) fn new(
        shared: Arc<Shared>,
        inputs: RuntimeInputs,
        commands: mpsc::Receiver<RuntimeCommand>,
        shutdown: watch::Receiver<bool>,
        backend: Box<dyn HidBackend>,
    ) -> Self {
        Self {
            shared,
            inputs,
            commands,
            shutdown,
            backend,
        }
    }

    pub(super) fn run(self) {
        let shared = self.shared.clone();
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let executor = tokio::runtime::Builder::new_current_thread()
                .build()
                .map_err(|error| format!("automation executor could not start: {error}"))?;
            executor.block_on(self.run_loop());
            Ok::<(), String>(())
        }));
        let error = match outcome {
            Ok(Ok(())) => return,
            Ok(Err(error)) => error,
            Err(_) => "automation runtime panicked".to_string(),
        };
        app_log::write_error(&error);
        shared.close_admission();
        shared.health.worker_failed(error);
    }

    async fn run_loop(mut self) {
        #[cfg(test)]
        self.shared.wait_before_initialization_claim();
        if !self.shared.claim_initialization(&self.inputs) {
            self.cancel_pending_commands(None);
            self.shared.health.mark_stopped();
            return;
        }
        self.refresh_hid();
        self.shared.health.startup_finished();

        let mut commands_open = true;
        let mut staged_command = None;
        loop {
            #[cfg(test)]
            if self.inputs.has_pending() {
                self.shared.wait_before_event_boundary_claim();
            }
            match self
                .shared
                .claim_boundary(&mut self.inputs, staged_command.is_some())
            {
                BoundaryClaim::Shutdown => {
                    self.cancel_pending_commands(staged_command.take());
                    break;
                }
                BoundaryClaim::Event { event, config } => {
                    self.dispatch_event(&event, config.as_deref());
                    self.inputs.mark_handled(event);
                    continue;
                }
                BoundaryClaim::Command => {
                    self.execute_command(
                        staged_command
                            .take()
                            .expect("boundary claimed a staged command"),
                    );
                    continue;
                }
                BoundaryClaim::Wait => {}
            }

            match self.commands.try_recv() {
                Ok(command) => {
                    staged_command = Some(command);
                    continue;
                }
                Err(mpsc::error::TryRecvError::Disconnected) => commands_open = false,
                Err(mpsc::error::TryRecvError::Empty) => {}
            }

            tokio::select! {
                biased;
                changed = self.shutdown.changed() => {
                    debug_assert!(changed.is_ok() && *self.shutdown.borrow_and_update());
                }
                source_change = self.inputs.changed() => {
                    if let Some((source, state)) = source_change {
                        self.shared.health.source_changed(source, state);
                    }
                }
                command = self.commands.recv(), if commands_open => {
                    match command {
                        Some(command) => staged_command = Some(command),
                        None => commands_open = false,
                    }
                }
            }
        }
        self.shared.health.mark_stopped();
    }

    fn execute_command(&mut self, command: RuntimeCommand) {
        match command {
            RuntimeCommand::TestAction {
                action,
                devices,
                response,
            } => {
                let result = self.dispatch_test(&action, &devices);
                let _ = response.send(Ok(result));
            }
            RuntimeCommand::RefreshHid => self.refresh_hid(),
        }
    }

    fn cancel_pending_commands(&mut self, staged: Option<RuntimeCommand>) {
        self.commands.close();
        if let Some(command) = staged {
            cancel_command(command);
        }
        while let Ok(command) = self.commands.try_recv() {
            cancel_command(command);
        }
        self.shared.close_admission();
    }

    fn refresh_hid(&mut self) {
        let mut refreshing = self.backend.inventory();
        refreshing.refresh_state = HidRefreshState::Refreshing;
        self.publish_hid_inventory(refreshing);

        let inventory = self.backend.refresh();
        {
            let mut admission = self
                .shared
                .admission
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            self.publish_hid_inventory(inventory.clone());
            admission.refresh_pending = false;
        }
        self.update_refresh_health(&inventory);
    }

    fn publish_hid_inventory(&self, inventory: HidInventory) {
        self.shared.hid_inventory.send_replace(Arc::new(inventory));
    }

    fn update_refresh_health(&self, inventory: &HidInventory) {
        let error = match &inventory.refresh_state {
            HidRefreshState::Ready => None,
            HidRefreshState::Failed { error } => Some(format!("HID discovery failed: {error}")),
            HidRefreshState::NotAttempted | HidRefreshState::Refreshing => {
                Some("HID discovery did not complete".to_string())
            }
        };
        if let Some(error) = &error {
            app_log::write_error(error);
        }
        self.shared.health.refresh_completed(error);
    }

    fn send_report(&mut self, device: &Device, report: &[u8]) -> Result<(), HidError> {
        let result = self.backend.send_report(device, report);
        let inventory = self.backend.inventory();
        let changed = self.shared.hid_inventory.borrow().as_ref() != &inventory;
        if changed {
            self.publish_hid_inventory(inventory.clone());
            self.update_refresh_health(&inventory);
        }
        result
    }

    fn dispatch_event(&mut self, event: &Event, config: Option<&ActiveConfig>) {
        let Some(config) = config else {
            return;
        };
        let mut attempted = false;
        let mut last_error = None;
        for evaluated in config.evaluate_event(event) {
            for device in evaluated.destinations() {
                attempted = true;
                match self.send_report(device, evaluated.report()) {
                    Ok(()) => app_log::write(format!(
                        "{} / {} sent {} bytes to {}",
                        evaluated.automation_name(),
                        evaluated.case_name(),
                        evaluated.report().len(),
                        device.name
                    )),
                    Err(error) => {
                        let message = format!(
                            "{} / {} failed for {}: {error:#}",
                            evaluated.automation_name(),
                            evaluated.case_name(),
                            device.name
                        );
                        app_log::write_error(&message);
                        last_error = Some(message);
                    }
                }
            }
        }
        if attempted {
            self.shared.health.dispatch_completed(last_error);
        }
    }

    fn dispatch_test(&mut self, action: &SendAction, devices: &[Device]) -> TestDispatchResult {
        let mut result = TestDispatchResult {
            sent: 0,
            failures: Vec::new(),
        };
        for device in devices {
            match self.send_report(device, &action.report) {
                Ok(()) => result.sent += 1,
                Err(error) => result.failures.push(format!("{}: {error}", device.name)),
            }
        }
        if !devices.is_empty() {
            let last_error = result
                .failures
                .last()
                .map(|error| format!("HID report failed: {error}"));
            self.shared.health.dispatch_completed(last_error);
        }
        result
    }
}

fn cancel_command(command: RuntimeCommand) {
    if let RuntimeCommand::TestAction { response, .. } = command {
        let _ = response.send(Err(RuntimeRequestError::Cancelled));
    }
}
