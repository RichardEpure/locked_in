use std::panic::{AssertUnwindSafe, catch_unwind};

use tokio::sync::{mpsc, watch};

use super::{
    AutomationRuntime, BoundaryClaim, RuntimeCommand, RuntimeLifecycle, RuntimeRequestError,
    TestDispatchResult,
};
use crate::{
    app_log,
    config::{ActiveConfig, Device, SendAction},
    focused_window::ForegroundObservation,
    hid::{HidBackend, HidError, HidInventory, HidRefreshState},
};

pub(super) fn run(
    runtime: AutomationRuntime,
    focus_events: watch::Receiver<ForegroundObservation>,
    commands: mpsc::Receiver<RuntimeCommand>,
    shutdown: watch::Receiver<bool>,
    backend: Box<dyn HidBackend>,
) {
    let panic_runtime = runtime.clone();
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let tokio_runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|error| format!("automation executor could not start: {error}"))?;
        tokio_runtime.block_on(run_loop(runtime, focus_events, commands, shutdown, backend));
        Ok::<(), String>(())
    }));
    match outcome {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            app_log::write_error(&error);
            panic_runtime.close_admission();
            panic_runtime.update_health(|health| {
                health.worker_error = Some(error);
                health.lifecycle = match health.lifecycle {
                    RuntimeLifecycle::Starting => RuntimeLifecycle::Running,
                    RuntimeLifecycle::Stopping => RuntimeLifecycle::Stopped,
                    lifecycle => lifecycle,
                };
            });
        }
        Err(_) => {
            app_log::write_error("automation runtime panicked");
            panic_runtime.close_admission();
            panic_runtime.update_health(|health| {
                health.worker_error = Some("automation runtime panicked".to_string());
                health.lifecycle = match health.lifecycle {
                    RuntimeLifecycle::Starting => RuntimeLifecycle::Running,
                    RuntimeLifecycle::Stopping => RuntimeLifecycle::Stopped,
                    lifecycle => lifecycle,
                };
            });
        }
    }
}

async fn run_loop(
    runtime: AutomationRuntime,
    mut focus_events: watch::Receiver<ForegroundObservation>,
    mut commands: mpsc::Receiver<RuntimeCommand>,
    mut shutdown: watch::Receiver<bool>,
    mut backend: Box<dyn HidBackend>,
) {
    runtime.wait_before_initialization_claim();
    if !runtime.claim_initialization() {
        cancel_pending_commands(&runtime, None, &mut commands);
        runtime.update_health(|health| {
            health.lifecycle = RuntimeLifecycle::Stopped;
        });
        return;
    }
    refresh_hid(&runtime, backend.as_mut(), true);

    let mut focus_open = true;
    let mut commands_open = true;
    let mut handled_focus_generation = 0;
    let mut staged_command = None;
    loop {
        #[cfg(test)]
        if focus_events.borrow().generation > handled_focus_generation {
            runtime.wait_before_focus_boundary_claim();
        }
        match runtime.claim_boundary(
            &mut focus_events,
            handled_focus_generation,
            staged_command.is_some(),
        ) {
            BoundaryClaim::Shutdown => {
                cancel_pending_commands(&runtime, staged_command.take(), &mut commands);
                break;
            }
            BoundaryClaim::Focus {
                observation,
                config,
            } => {
                dispatch_focus(&runtime, backend.as_mut(), &observation, config.as_deref());
                handled_focus_generation = observation.generation;
                runtime.mark_focus_handled(observation.generation);
                continue;
            }
            BoundaryClaim::Command => {
                execute_command(
                    &runtime,
                    backend.as_mut(),
                    staged_command
                        .take()
                        .expect("boundary claimed a staged command"),
                );
                continue;
            }
            BoundaryClaim::Wait => {}
        }

        match commands.try_recv() {
            Ok(command) => {
                staged_command = Some(command);
                continue;
            }
            Err(mpsc::error::TryRecvError::Disconnected) => commands_open = false,
            Err(mpsc::error::TryRecvError::Empty) => {}
        }

        tokio::select! {
            biased;
            changed = shutdown.changed() => {
                debug_assert!(changed.is_ok() && *shutdown.borrow_and_update());
            }
            changed = focus_events.changed(), if focus_open => {
                if changed.is_err() {
                    focus_open = false;
                    runtime.update_health(|health| {
                        health.focus_error = Some("focus event source closed".to_string());
                    });
                }
            }
            command = commands.recv(), if commands_open => {
                match command {
                    Some(command) => staged_command = Some(command),
                    None => commands_open = false,
                }
            }
        }
    }

    runtime.update_health(|health| {
        health.lifecycle = RuntimeLifecycle::Stopped;
    });
}

fn execute_command(
    runtime: &AutomationRuntime,
    backend: &mut dyn HidBackend,
    command: RuntimeCommand,
) {
    match command {
        RuntimeCommand::TestAction {
            action,
            devices,
            response,
        } => {
            let result = dispatch_test(runtime, backend, &action, &devices);
            let _ = response.send(Ok(result));
        }
        RuntimeCommand::RefreshHid => {
            refresh_hid(runtime, backend, false);
        }
    }
}

fn cancel_pending_commands(
    runtime: &AutomationRuntime,
    staged: Option<RuntimeCommand>,
    commands: &mut mpsc::Receiver<RuntimeCommand>,
) {
    commands.close();
    if let Some(command) = staged {
        cancel_command(command);
    }
    while let Ok(command) = commands.try_recv() {
        cancel_command(command);
    }
    runtime.close_admission();
}

fn cancel_command(command: RuntimeCommand) {
    if let RuntimeCommand::TestAction { response, .. } = command {
        let _ = response.send(Err(RuntimeRequestError::Cancelled));
    }
}

fn refresh_hid(runtime: &AutomationRuntime, backend: &mut dyn HidBackend, startup: bool) {
    let mut refreshing = backend.inventory();
    refreshing.refresh_state = HidRefreshState::Refreshing;
    runtime.publish_hid_inventory(refreshing);

    let inventory = backend.refresh();
    runtime.publish_completed_hid_refresh(inventory.clone());
    update_refresh_health(runtime, &inventory);
    if startup {
        runtime.update_health(|health| {
            if health.lifecycle == RuntimeLifecycle::Starting {
                health.lifecycle = RuntimeLifecycle::Running;
            }
        });
    }
}

fn update_refresh_health(runtime: &AutomationRuntime, inventory: &HidInventory) {
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
    runtime.update_health(|health| health.refresh_error = error);
}

fn send_report(
    runtime: &AutomationRuntime,
    backend: &mut dyn HidBackend,
    device: &Device,
    report: &[u8],
) -> Result<(), HidError> {
    let result = backend.send_report(device, report);
    let inventory = backend.inventory();
    if runtime.publish_hid_inventory_if_changed(inventory.clone()) {
        update_refresh_health(runtime, &inventory);
    }
    result
}

fn dispatch_focus(
    runtime: &AutomationRuntime,
    backend: &mut dyn HidBackend,
    focused: &ForegroundObservation,
    config: Option<&ActiveConfig>,
) {
    let Some(config) = config else {
        return;
    };
    let mut attempted = false;
    let mut last_error = None;
    for evaluated in config.evaluate_window(&focused.window) {
        for device in evaluated.destinations() {
            attempted = true;
            match send_report(runtime, backend, device, evaluated.report()) {
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
    update_dispatch_health(runtime, attempted, last_error);
}

fn dispatch_test(
    runtime: &AutomationRuntime,
    backend: &mut dyn HidBackend,
    action: &SendAction,
    devices: &[Device],
) -> TestDispatchResult {
    let mut result = TestDispatchResult {
        sent: 0,
        failures: Vec::new(),
    };
    for device in devices {
        match send_report(runtime, backend, device, &action.report) {
            Ok(()) => result.sent += 1,
            Err(error) => result.failures.push(format!("{}: {error}", device.name)),
        }
    }
    let last_error = result
        .failures
        .last()
        .map(|error| format!("HID report failed: {error}"));
    update_dispatch_health(runtime, !devices.is_empty(), last_error);
    result
}

fn update_dispatch_health(runtime: &AutomationRuntime, attempted: bool, error: Option<String>) {
    if attempted {
        runtime.update_health(|health| health.dispatch_error = error);
    }
}
