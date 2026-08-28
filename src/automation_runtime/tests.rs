use std::sync::{Arc, Mutex, mpsc};

use super::{
    AutomationRuntime, FocusSourceState, ReportDispatcher, RuntimePhase, TestDispatchResult,
};
use crate::{
    config::{
        Automation, AutomationCase, Config, Device, SendAction, TextCondition, WindowMatcher,
    },
    win::WindowMetadata,
};

struct RecordingDispatcher {
    calls: mpsc::Sender<(String, Vec<u8>)>,
    initialize_error: Option<String>,
    failed_devices: Arc<Mutex<Vec<String>>>,
    first_call_gate: Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>,
}

impl ReportDispatcher for RecordingDispatcher {
    fn initialize(&mut self) -> anyhow::Result<()> {
        if let Some(error) = &self.initialize_error {
            anyhow::bail!(error.clone());
        }
        Ok(())
    }

    fn send_report(&mut self, device: &Device, report: &[u8]) -> anyhow::Result<usize> {
        self.calls
            .send((device.id.clone(), report.to_vec()))
            .unwrap();
        if let Some((started, release)) = self.first_call_gate.take() {
            started.send(()).unwrap();
            release.recv().unwrap();
        }
        if self.failed_devices.lock().unwrap().contains(&device.id) {
            anyhow::bail!("configured failure");
        }
        Ok(report.len())
    }
}

fn device(id: &str) -> Device {
    Device {
        id: id.to_string(),
        name: id.to_string(),
        report_length: 32,
        ..Device::default()
    }
}

fn config(report: u8, device_ids: &[&str]) -> Config {
    let devices = device_ids.iter().map(|id| device(id)).collect::<Vec<_>>();
    Config {
        devices,
        automations: vec![Automation {
            id: "automation".to_string(),
            name: "Automation".to_string(),
            enabled: true,
            cases: vec![AutomationCase {
                id: "case".to_string(),
                name: "Case".to_string(),
                applications: vec![WindowMatcher {
                    id: "matcher".to_string(),
                    title: Some(TextCondition::contains("target")),
                    ..WindowMatcher::default()
                }],
                actions: vec![SendAction {
                    id: "action".to_string(),
                    report: vec![report],
                    device_ids: device_ids.iter().map(|id| (*id).to_string()).collect(),
                    ..SendAction::default()
                }],
                ..AutomationCase::default()
            }],
            ..Automation::default()
        }],
        ..Config::default()
    }
}

fn focused(title: &str) -> WindowMetadata {
    WindowMetadata {
        title: Some(title.to_string()),
        ..WindowMetadata::default()
    }
}

fn dispatcher(
    initialize_error: Option<&str>,
) -> (RecordingDispatcher, mpsc::Receiver<(String, Vec<u8>)>) {
    let (calls, received) = mpsc::channel();
    (
        RecordingDispatcher {
            calls,
            initialize_error: initialize_error.map(str::to_string),
            failed_devices: Arc::new(Mutex::new(Vec::new())),
            first_call_gate: None,
        },
        received,
    )
}

#[test]
fn event_waiting_before_worker_start_is_dispatched_in_device_order() {
    let (focus_tx, focus_rx) = tokio::sync::watch::channel(WindowMetadata::default());
    focus_tx.send_replace(focused("target"));
    let (dispatcher, calls) = dispatcher(None);
    let (runtime, owner) = AutomationRuntime::start(
        Some(config(0x87, &["one", "two"])),
        focus_rx,
        FocusSourceState::Available,
        dispatcher,
    )
    .unwrap();

    assert_eq!(calls.recv().unwrap(), ("one".to_string(), vec![0x87]));
    assert_eq!(calls.recv().unwrap(), ("two".to_string(), vec![0x87]));
    assert_eq!(runtime.status().phase, RuntimePhase::Active);
    owner.shutdown_and_join(std::time::Duration::from_secs(1));
}

#[test]
fn device_failure_does_not_stop_later_destinations() {
    let (focus_tx, focus_rx) = tokio::sync::watch::channel(WindowMetadata::default());
    let (dispatcher, calls) = dispatcher(None);
    dispatcher
        .failed_devices
        .lock()
        .unwrap()
        .push("one".to_string());
    let (runtime, owner) = AutomationRuntime::start(
        Some(config(0x87, &["one", "two"])),
        focus_rx,
        FocusSourceState::Available,
        dispatcher,
    )
    .unwrap();

    focus_tx.send_replace(focused("target"));
    assert_eq!(calls.recv().unwrap().0, "one");
    assert_eq!(calls.recv().unwrap().0, "two");
    let mut status = runtime.subscribe_status();
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    tokio_runtime.block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while status.borrow().phase != RuntimePhase::Degraded {
                status.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
    });
    owner.shutdown_and_join(std::time::Duration::from_secs(1));
}

#[test]
fn in_flight_event_keeps_its_snapshot_and_next_event_uses_replacement() {
    let (focus_tx, focus_rx) = tokio::sync::watch::channel(WindowMetadata::default());
    let (calls_tx, calls) = mpsc::channel();
    let (started_tx, started) = mpsc::channel();
    let (release_tx, release) = mpsc::channel();
    let dispatcher = RecordingDispatcher {
        calls: calls_tx,
        initialize_error: None,
        failed_devices: Arc::new(Mutex::new(Vec::new())),
        first_call_gate: Some((started_tx, release)),
    };
    let (runtime, owner) = AutomationRuntime::start(
        Some(config(0x10, &["one", "two"])),
        focus_rx,
        FocusSourceState::Available,
        dispatcher,
    )
    .unwrap();

    focus_tx.send_replace(focused("target one"));
    started.recv().unwrap();
    runtime.replace_config(config(0x20, &["one"]));
    release_tx.send(()).unwrap();
    assert_eq!(calls.recv().unwrap(), ("one".to_string(), vec![0x10]));
    assert_eq!(calls.recv().unwrap(), ("two".to_string(), vec![0x10]));

    focus_tx.send_replace(focused("target two"));
    assert_eq!(calls.recv().unwrap(), ("one".to_string(), vec![0x20]));
    owner.shutdown_and_join(std::time::Duration::from_secs(1));
}

#[test]
fn blocked_dispatch_coalesces_focus_changes_to_the_latest() {
    let (focus_tx, focus_rx) = tokio::sync::watch::channel(WindowMetadata::default());
    let (calls_tx, calls) = mpsc::channel();
    let (started_tx, started) = mpsc::channel();
    let (release_tx, release) = mpsc::channel();
    let dispatcher = RecordingDispatcher {
        calls: calls_tx,
        initialize_error: None,
        failed_devices: Arc::new(Mutex::new(Vec::new())),
        first_call_gate: Some((started_tx, release)),
    };
    let (runtime, owner) = AutomationRuntime::start(
        Some(config(0x10, &["one"])),
        focus_rx,
        FocusSourceState::Available,
        dispatcher,
    )
    .unwrap();

    focus_tx.send_replace(focused("target first"));
    started.recv().unwrap();
    runtime.replace_config(config(0x20, &["one"]));
    focus_tx.send_replace(focused("not matching"));
    focus_tx.send_replace(focused("target latest"));
    release_tx.send(()).unwrap();

    assert_eq!(calls.recv().unwrap().1, vec![0x10]);
    assert_eq!(calls.recv().unwrap().1, vec![0x20]);
    runtime.request_shutdown();
    owner.shutdown_and_join(std::time::Duration::from_secs(1));
    assert!(calls.try_recv().is_err());
}

#[test]
fn hid_initialization_failure_is_degraded_and_test_dispatch_still_runs() {
    let (_focus_tx, focus_rx) = tokio::sync::watch::channel(WindowMetadata::default());
    let (dispatcher, calls) = dispatcher(Some("not available"));
    let (runtime, owner) = AutomationRuntime::start(
        Some(config(0x87, &["one"])),
        focus_rx,
        FocusSourceState::Available,
        dispatcher,
    )
    .unwrap();
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let mut status = runtime.subscribe_status();
    tokio_runtime.block_on(async {
        while status.borrow().phase == RuntimePhase::Starting {
            status.changed().await.unwrap();
        }
    });
    assert_eq!(status.borrow().phase, RuntimePhase::Degraded);
    let result: TestDispatchResult = tokio_runtime
        .block_on(runtime.test_action(
            SendAction {
                report: vec![0x44],
                ..SendAction::default()
            },
            vec![device("one")],
        ))
        .unwrap();

    assert_eq!(calls.recv().unwrap().1, vec![0x44]);
    assert_eq!(result.sent, 1);
    assert!(result.failures.is_empty());
    assert_eq!(runtime.status().phase, RuntimePhase::Active);
    owner.shutdown_and_join(std::time::Duration::from_secs(1));
}

#[test]
fn missing_focus_source_or_configuration_is_unavailable() {
    let (_focus_tx, focus_rx) = tokio::sync::watch::channel(WindowMetadata::default());
    let (dispatcher, _) = dispatcher(None);
    let (runtime, owner) = AutomationRuntime::start(
        None,
        focus_rx,
        FocusSourceState::Unavailable("hook failed".to_string()),
        dispatcher,
    )
    .unwrap();

    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let mut status = runtime.subscribe_status();
    tokio_runtime.block_on(async {
        while status.borrow().phase == RuntimePhase::Starting {
            status.changed().await.unwrap();
        }
    });
    assert_eq!(status.borrow().phase, RuntimePhase::Unavailable);
    assert!(
        status
            .borrow()
            .detail
            .as_ref()
            .unwrap()
            .contains("hook failed")
    );
    runtime.request_shutdown();
    runtime.request_shutdown();
    owner.shutdown_and_join(std::time::Duration::from_secs(1));
    assert_eq!(runtime.status().phase, RuntimePhase::Stopped);
}

#[test]
fn shutdown_reports_stopping_until_in_flight_dispatch_completes() {
    let (focus_tx, focus_rx) = tokio::sync::watch::channel(WindowMetadata::default());
    let (calls_tx, _calls) = mpsc::channel();
    let (started_tx, started) = mpsc::channel();
    let (release_tx, release) = mpsc::channel();
    let dispatcher = RecordingDispatcher {
        calls: calls_tx,
        initialize_error: None,
        failed_devices: Arc::new(Mutex::new(Vec::new())),
        first_call_gate: Some((started_tx, release)),
    };
    let (runtime, owner) = AutomationRuntime::start(
        Some(config(0x87, &["one"])),
        focus_rx,
        FocusSourceState::Available,
        dispatcher,
    )
    .unwrap();

    focus_tx.send_replace(focused("target"));
    started.recv().unwrap();
    runtime.request_shutdown();
    assert_eq!(runtime.status().phase, RuntimePhase::Stopping);
    release_tx.send(()).unwrap();
    owner.shutdown_and_join(std::time::Duration::from_secs(1));
    assert_eq!(runtime.status().phase, RuntimePhase::Stopped);
}
