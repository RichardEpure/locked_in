use std::panic::{AssertUnwindSafe, catch_unwind};

use tokio::sync::{mpsc, watch};

use super::{AutomationRuntime, ReportDispatcher, RuntimeCommand, TestDispatchResult};
use crate::{app_log, win::WindowMetadata};

pub(super) fn run(
    runtime: AutomationRuntime,
    focus_events: watch::Receiver<WindowMetadata>,
    commands: mpsc::UnboundedReceiver<RuntimeCommand>,
    dispatcher: Box<dyn ReportDispatcher>,
) {
    let panic_runtime = runtime.clone();
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let tokio_runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|error| format!("automation executor could not start: {error}"))?;
        tokio_runtime.block_on(run_loop(runtime, focus_events, commands, dispatcher));
        Ok::<(), String>(())
    }));
    match outcome {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            app_log::write_error(&error);
            panic_runtime.update_health(|health| {
                health.worker_error = Some(error);
                health.started = true;
            });
        }
        Err(_) => {
            app_log::write_error("automation runtime panicked");
            panic_runtime.update_health(|health| {
                health.worker_error = Some("automation runtime panicked".to_string());
                health.started = true;
            });
        }
    }
}

async fn run_loop(
    runtime: AutomationRuntime,
    mut focus_events: watch::Receiver<WindowMetadata>,
    mut commands: mpsc::UnboundedReceiver<RuntimeCommand>,
    mut dispatcher: Box<dyn ReportDispatcher>,
) {
    match dispatcher.initialize() {
        Ok(()) => runtime.update_health(|health| {
            health.hid_error = None;
            health.started = true;
        }),
        Err(error) => {
            let message = format!("HID discovery failed: {error:#}");
            app_log::write_error(&message);
            runtime.update_health(|health| {
                health.hid_error = Some(message);
                health.started = true;
            });
        }
    }

    let mut focus_open = true;
    loop {
        tokio::select! {
            biased;
            command = commands.recv() => {
                match command {
                    Some(RuntimeCommand::TestAction { action, devices, response }) => {
                        let result = dispatch_test(&runtime, dispatcher.as_mut(), &action, &devices);
                        let _ = response.send(result);
                    }
                    Some(RuntimeCommand::Shutdown) | None => break,
                }
            }
            changed = focus_events.changed(), if focus_open => {
                if changed.is_err() {
                    focus_open = false;
                    runtime.update_health(|health| {
                        health.focus_error = Some("focus event source closed".to_string());
                    });
                    continue;
                }
                let focused = focus_events.borrow_and_update().clone();
                dispatch_focus(&runtime, dispatcher.as_mut(), &focused);
            }
        }
    }

    runtime.update_health(|health| {
        health.stopping = false;
        health.stopped = true;
    });
}

fn dispatch_focus(
    runtime: &AutomationRuntime,
    dispatcher: &mut dyn ReportDispatcher,
    focused: &WindowMetadata,
) {
    let Some(config) = runtime.config_snapshot() else {
        return;
    };
    let mut attempted = false;
    let mut last_error = None;
    for evaluated in config.evaluate_window(focused) {
        for device in evaluated.devices {
            attempted = true;
            match dispatcher.send_report(device, &evaluated.action.report) {
                Ok(_) => app_log::write(format!(
                    "{} / {} sent {} bytes to {}",
                    evaluated.automation_name,
                    evaluated.case_name,
                    evaluated.action.report.len(),
                    device.name
                )),
                Err(error) => {
                    let message = format!(
                        "{} / {} failed for {}: {error:#}",
                        evaluated.automation_name, evaluated.case_name, device.name
                    );
                    app_log::write_error(&message);
                    last_error = Some(message);
                }
            }
        }
    }
    update_hid_health(runtime, attempted, last_error);
}

fn dispatch_test(
    runtime: &AutomationRuntime,
    dispatcher: &mut dyn ReportDispatcher,
    action: &crate::config::SendAction,
    devices: &[crate::config::Device],
) -> TestDispatchResult {
    let mut result = TestDispatchResult {
        sent: 0,
        failures: Vec::new(),
    };
    for device in devices {
        match dispatcher.send_report(device, &action.report) {
            Ok(_) => result.sent += 1,
            Err(error) => result.failures.push(format!("{}: {error}", device.name)),
        }
    }
    let last_error = result
        .failures
        .last()
        .map(|error| format!("HID report failed: {error}"));
    update_hid_health(runtime, !devices.is_empty(), last_error);
    result
}

fn update_hid_health(runtime: &AutomationRuntime, attempted: bool, error: Option<String>) {
    if attempted {
        runtime.update_health(|health| health.hid_error = error);
    }
}
