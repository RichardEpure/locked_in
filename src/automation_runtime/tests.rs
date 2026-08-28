use std::{
    collections::VecDeque,
    future::Future,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

use super::{
    AutomationRuntime, FocusSourceState, HidRefreshRequestResult, RuntimeOwner, RuntimePhase,
    TestDispatchResult,
};
use crate::{
    config::{
        Automation, AutomationCase, Config, Device, SendAction, TextCondition, WindowMatcher,
    },
    hid::{HidBackend, HidError, HidInventory, HidRefreshState},
    win::WindowMetadata,
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum BackendEvent {
    RefreshStarted,
    RefreshFinished(u64),
    Send(String, Vec<u8>),
}

type Gate = (mpsc::Sender<()>, mpsc::Receiver<()>);

struct RecordingBackend {
    events: mpsc::Sender<BackendEvent>,
    inventory: HidInventory,
    refresh_results: VecDeque<HidInventory>,
    refresh_gates: VecDeque<Option<Gate>>,
    failed_devices: Arc<Mutex<Vec<String>>>,
    first_send_gate: Option<Gate>,
    send_inventories: VecDeque<HidInventory>,
    panic_on_refresh: bool,
}

impl HidBackend for RecordingBackend {
    fn inventory(&self) -> HidInventory {
        self.inventory.clone()
    }

    fn refresh(&mut self) -> HidInventory {
        assert!(!self.panic_on_refresh, "configured refresh panic");
        self.events.send(BackendEvent::RefreshStarted).unwrap();
        if let Some(Some((started, release))) = self.refresh_gates.pop_front() {
            started.send(()).unwrap();
            release.recv().unwrap();
        }
        self.inventory = self.refresh_results.pop_front().unwrap_or_else(|| {
            let mut inventory = self.inventory.clone();
            inventory.revision = inventory.revision.wrapping_add(1);
            inventory.refresh_state = HidRefreshState::Ready;
            inventory
        });
        self.events
            .send(BackendEvent::RefreshFinished(self.inventory.revision))
            .unwrap();
        self.inventory.clone()
    }

    fn send_report(&mut self, device: &Device, report: &[u8]) -> Result<(), HidError> {
        self.events
            .send(BackendEvent::Send(device.id.clone(), report.to_vec()))
            .unwrap();
        if let Some((started, release)) = self.first_send_gate.take() {
            started.send(()).unwrap();
            release.recv().unwrap();
        }
        if let Some(inventory) = self.send_inventories.pop_front() {
            self.inventory = inventory;
        }
        if self.failed_devices.lock().unwrap().contains(&device.id) {
            return Err(HidError::Write {
                selector: device.into(),
                message: "configured failure".into(),
            });
        }
        Ok(())
    }
}

fn ready(revision: u64) -> HidInventory {
    HidInventory {
        revision,
        refresh_state: HidRefreshState::Ready,
        rows: Vec::new(),
    }
}

fn failed(revision: u64, message: &str) -> HidInventory {
    HidInventory {
        revision,
        refresh_state: HidRefreshState::Failed {
            error: HidError::Enumeration {
                message: message.to_string(),
            },
        },
        rows: Vec::new(),
    }
}

fn backend(
    refresh_results: impl IntoIterator<Item = HidInventory>,
) -> (RecordingBackend, mpsc::Receiver<BackendEvent>) {
    let (events, received) = mpsc::channel();
    (
        RecordingBackend {
            events,
            inventory: HidInventory::default(),
            refresh_results: refresh_results.into_iter().collect(),
            refresh_gates: VecDeque::new(),
            failed_devices: Arc::new(Mutex::new(Vec::new())),
            first_send_gate: None,
            send_inventories: VecDeque::new(),
            panic_on_refresh: false,
        },
        received,
    )
}

fn device(id: &str) -> Device {
    Device {
        id: id.to_string(),
        name: id.to_string(),
        report_length: 32,
        ..Device::default()
    }
}

fn action(report: u8) -> SendAction {
    SendAction {
        report: vec![report],
        ..SendAction::default()
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

fn start(
    config: Option<Config>,
    focus_rx: tokio::sync::watch::Receiver<WindowMetadata>,
    backend: RecordingBackend,
) -> (AutomationRuntime, RuntimeOwner) {
    AutomationRuntime::start(config, focus_rx, FocusSourceState::Available, backend).unwrap()
}

fn block_on<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(1);
    while !condition() {
        assert!(Instant::now() < deadline, "condition timed out");
        thread::sleep(Duration::from_millis(1));
    }
}

fn wait_for_phase(runtime: &AutomationRuntime, phase: RuntimePhase) {
    wait_until(|| runtime.status().phase == phase);
}

fn wait_for_revision(runtime: &AutomationRuntime, revision: u64) {
    wait_until(|| runtime.hid_inventory().revision == revision);
}

fn finish_startup(
    runtime: &AutomationRuntime,
    events: &mpsc::Receiver<BackendEvent>,
    revision: u64,
) {
    assert_eq!(events.recv().unwrap(), BackendEvent::RefreshStarted);
    assert_eq!(
        events.recv().unwrap(),
        BackendEvent::RefreshFinished(revision)
    );
    wait_for_revision(runtime, revision);
    wait_until(|| runtime.status().phase != RuntimePhase::Starting);
}

#[test]
fn startup_publishes_refreshing_before_the_final_inventory_and_status() {
    let (_focus_tx, focus_rx) = tokio::sync::watch::channel(WindowMetadata::default());
    let (mut backend, events) = backend([ready(1), ready(2)]);
    let (started_tx, started) = mpsc::channel();
    let (release_tx, release) = mpsc::channel();
    backend.refresh_gates.push_back(Some((started_tx, release)));
    let (runtime, owner) = start(Some(config(1, &["one"])), focus_rx, backend);
    let inventory = runtime.subscribe_hid_inventory();

    started.recv().unwrap();
    assert_eq!(events.recv().unwrap(), BackendEvent::RefreshStarted);
    assert_eq!(
        inventory.borrow().refresh_state,
        HidRefreshState::Refreshing
    );
    assert_eq!(runtime.status().phase, RuntimePhase::Starting);
    assert_eq!(
        runtime.request_hid_refresh().unwrap(),
        HidRefreshRequestResult::AlreadyPending
    );

    release_tx.send(()).unwrap();
    assert_eq!(events.recv().unwrap(), BackendEvent::RefreshFinished(1));
    wait_for_revision(&runtime, 1);
    wait_for_phase(&runtime, RuntimePhase::Active);
    assert_eq!(
        runtime.hid_inventory().refresh_state,
        HidRefreshState::Ready
    );

    assert_eq!(
        runtime.request_hid_refresh().unwrap(),
        HidRefreshRequestResult::Queued
    );
    assert_eq!(events.recv().unwrap(), BackendEvent::RefreshStarted);
    assert_eq!(events.recv().unwrap(), BackendEvent::RefreshFinished(2));
    wait_for_revision(&runtime, 2);
    owner.shutdown_and_join(Duration::from_secs(1));
}

#[test]
fn admitted_command_precedes_focus_pending_at_the_same_worker_boundary() {
    let (focus_tx, focus_rx) = tokio::sync::watch::channel(WindowMetadata::default());
    let (mut backend, events) = backend([ready(1)]);
    let (refresh_started_tx, refresh_started) = mpsc::channel();
    let (refresh_release_tx, refresh_release) = mpsc::channel();
    backend
        .refresh_gates
        .push_back(Some((refresh_started_tx, refresh_release)));
    let (runtime, owner) = start(Some(config(0x10, &["automatic"])), focus_rx, backend);

    refresh_started.recv().unwrap();
    assert_eq!(events.recv().unwrap(), BackendEvent::RefreshStarted);
    focus_tx.send_replace(focused("target"));

    block_on(async {
        let test_runtime = runtime.clone();
        let test = tokio::spawn(async move {
            test_runtime
                .test_action(action(0x20), vec![device("manual")])
                .await
                .unwrap()
        });
        tokio::task::yield_now().await;
        refresh_release_tx.send(()).unwrap();

        assert_eq!(events.recv().unwrap(), BackendEvent::RefreshFinished(1));
        assert_eq!(
            events.recv().unwrap(),
            BackendEvent::Send("manual".to_string(), vec![0x20])
        );
        assert_eq!(
            events.recv().unwrap(),
            BackendEvent::Send("automatic".to_string(), vec![0x10])
        );
        assert_eq!(test.await.unwrap().sent, 1);
    });

    owner.shutdown_and_join(Duration::from_secs(1));
}

#[test]
fn startup_failure_stays_degraded_after_test_and_explicit_refresh_recovers() {
    let (_focus_tx, focus_rx) = tokio::sync::watch::channel(WindowMetadata::default());
    let (mut backend, events) = backend([failed(1, "startup"), ready(2)]);
    backend.refresh_gates.push_back(None);
    let (started_tx, started) = mpsc::channel();
    let (release_tx, release) = mpsc::channel();
    backend.refresh_gates.push_back(Some((started_tx, release)));
    let (runtime, owner) = start(Some(config(1, &["one"])), focus_rx, backend);

    finish_startup(&runtime, &events, 1);
    assert_eq!(runtime.status().phase, RuntimePhase::Degraded);
    let result = block_on(runtime.test_action(action(0x44), vec![device("one")])).unwrap();
    assert_eq!(result.sent, 1);
    assert_eq!(
        events.recv().unwrap(),
        BackendEvent::Send("one".to_string(), vec![0x44])
    );
    assert_eq!(runtime.status().phase, RuntimePhase::Degraded);
    assert_eq!(
        runtime.request_hid_refresh().unwrap(),
        HidRefreshRequestResult::Queued
    );
    started.recv().unwrap();
    assert_eq!(events.recv().unwrap(), BackendEvent::RefreshStarted);
    assert_eq!(
        runtime.hid_inventory().refresh_state,
        HidRefreshState::Refreshing
    );
    assert_eq!(runtime.status().phase, RuntimePhase::Degraded);

    release_tx.send(()).unwrap();
    assert_eq!(events.recv().unwrap(), BackendEvent::RefreshFinished(2));
    wait_for_revision(&runtime, 2);
    wait_for_phase(&runtime, RuntimePhase::Active);
    owner.shutdown_and_join(Duration::from_secs(1));
}

#[test]
fn refresh_requests_coalesce_until_completion_then_allow_another_request() {
    let (_focus_tx, focus_rx) = tokio::sync::watch::channel(WindowMetadata::default());
    let (mut backend, events) = backend([ready(1), ready(2), ready(3)]);
    let (send_started_tx, send_started) = mpsc::channel();
    let (send_release_tx, send_release) = mpsc::channel();
    backend.first_send_gate = Some((send_started_tx, send_release));
    backend.refresh_gates.push_back(None);
    let (refresh_started_tx, refresh_started) = mpsc::channel();
    let (refresh_release_tx, refresh_release) = mpsc::channel();
    backend
        .refresh_gates
        .push_back(Some((refresh_started_tx, refresh_release)));
    let (runtime, owner) = start(Some(config(1, &["one"])), focus_rx, backend);
    finish_startup(&runtime, &events, 1);

    let test_runtime = runtime.clone();
    let test = thread::spawn(move || {
        block_on(test_runtime.test_action(action(1), vec![device("one")])).unwrap()
    });
    send_started.recv().unwrap();
    assert!(matches!(events.recv().unwrap(), BackendEvent::Send(..)));
    assert_eq!(
        runtime.request_hid_refresh().unwrap(),
        HidRefreshRequestResult::Queued
    );
    assert_eq!(
        runtime.request_hid_refresh().unwrap(),
        HidRefreshRequestResult::AlreadyPending
    );

    send_release_tx.send(()).unwrap();
    refresh_started.recv().unwrap();
    assert_eq!(events.recv().unwrap(), BackendEvent::RefreshStarted);
    assert_eq!(
        runtime.request_hid_refresh().unwrap(),
        HidRefreshRequestResult::AlreadyPending
    );
    assert_eq!(
        runtime.hid_inventory().refresh_state,
        HidRefreshState::Refreshing
    );

    refresh_release_tx.send(()).unwrap();
    assert_eq!(events.recv().unwrap(), BackendEvent::RefreshFinished(2));
    wait_for_revision(&runtime, 2);
    assert_eq!(
        runtime.request_hid_refresh().unwrap(),
        HidRefreshRequestResult::Queued
    );
    assert_eq!(events.recv().unwrap(), BackendEvent::RefreshStarted);
    assert_eq!(events.recv().unwrap(), BackendEvent::RefreshFinished(3));
    wait_for_revision(&runtime, 3);
    assert_eq!(test.join().unwrap().sent, 1);
    owner.shutdown_and_join(Duration::from_secs(1));
}

#[test]
fn admitted_test_refresh_test_and_shutdown_execute_fifo_without_interleaving() {
    let (_focus_tx, focus_rx) = tokio::sync::watch::channel(WindowMetadata::default());
    let (mut backend, events) = backend([ready(1), ready(2)]);
    let (send_started_tx, send_started) = mpsc::channel();
    let (send_release_tx, send_release) = mpsc::channel();
    backend.first_send_gate = Some((send_started_tx, send_release));
    let (runtime, owner) = start(Some(config(1, &["one"])), focus_rx, backend);
    finish_startup(&runtime, &events, 1);

    block_on(async {
        let first_runtime = runtime.clone();
        let first = tokio::spawn(async move {
            first_runtime
                .test_action(action(1), vec![device("one"), device("two")])
                .await
                .unwrap()
        });
        tokio::task::yield_now().await;
        send_started.recv().unwrap();
        assert_eq!(
            events.recv().unwrap(),
            BackendEvent::Send("one".to_string(), vec![1])
        );
        assert_eq!(
            runtime.request_hid_refresh().unwrap(),
            HidRefreshRequestResult::Queued
        );

        let second_runtime = runtime.clone();
        let second = tokio::spawn(async move {
            second_runtime
                .test_action(action(2), vec![device("three")])
                .await
                .unwrap()
        });
        tokio::task::yield_now().await;
        runtime.request_shutdown();
        runtime.request_shutdown();
        assert!(runtime.request_hid_refresh().is_err());
        assert!(
            runtime
                .test_action(action(3), vec![device("late")])
                .await
                .is_err()
        );

        send_release_tx.send(()).unwrap();
        assert_eq!(
            events.recv().unwrap(),
            BackendEvent::Send("two".to_string(), vec![1])
        );
        assert_eq!(events.recv().unwrap(), BackendEvent::RefreshStarted);
        assert_eq!(events.recv().unwrap(), BackendEvent::RefreshFinished(2));
        assert_eq!(
            events.recv().unwrap(),
            BackendEvent::Send("three".to_string(), vec![2])
        );
        assert_eq!(first.await.unwrap().sent, 2);
        assert_eq!(second.await.unwrap().sent, 1);
    });

    owner.shutdown_and_join(Duration::from_secs(1));
    assert_eq!(runtime.status().phase, RuntimePhase::Stopped);
    assert!(events.try_recv().is_err());
}

#[test]
fn automatic_and_manual_batches_do_not_interleave() {
    let (focus_tx, focus_rx) = tokio::sync::watch::channel(WindowMetadata::default());
    let (mut backend, events) = backend([ready(1)]);
    let (send_started_tx, send_started) = mpsc::channel();
    let (send_release_tx, send_release) = mpsc::channel();
    backend.first_send_gate = Some((send_started_tx, send_release));
    let (runtime, owner) = start(Some(config(0x10, &["one", "two"])), focus_rx, backend);
    finish_startup(&runtime, &events, 1);

    focus_tx.send_replace(focused("target"));
    send_started.recv().unwrap();
    assert_eq!(
        events.recv().unwrap(),
        BackendEvent::Send("one".to_string(), vec![0x10])
    );
    let test_runtime = runtime.clone();
    let test = thread::spawn(move || {
        block_on(test_runtime.test_action(action(0x20), vec![device("manual")])).unwrap()
    });
    send_release_tx.send(()).unwrap();

    assert_eq!(
        events.recv().unwrap(),
        BackendEvent::Send("two".to_string(), vec![0x10])
    );
    assert_eq!(
        events.recv().unwrap(),
        BackendEvent::Send("manual".to_string(), vec![0x20])
    );
    assert_eq!(test.join().unwrap().sent, 1);
    owner.shutdown_and_join(Duration::from_secs(1));
}

#[test]
fn blocked_dispatch_keeps_its_snapshot_and_uses_only_latest_pending_focus() {
    let (focus_tx, focus_rx) = tokio::sync::watch::channel(WindowMetadata::default());
    let (mut backend, events) = backend([ready(1)]);
    let (send_started_tx, send_started) = mpsc::channel();
    let (send_release_tx, send_release) = mpsc::channel();
    backend.first_send_gate = Some((send_started_tx, send_release));
    let (runtime, owner) = start(Some(config(0x10, &["one", "two"])), focus_rx, backend);
    finish_startup(&runtime, &events, 1);

    focus_tx.send_replace(focused("target first"));
    send_started.recv().unwrap();
    assert_eq!(
        events.recv().unwrap(),
        BackendEvent::Send("one".to_string(), vec![0x10])
    );
    runtime.replace_config(config(0x20, &["one"]));
    focus_tx.send_replace(focused("not matching"));
    focus_tx.send_replace(focused("target latest"));
    send_release_tx.send(()).unwrap();

    assert_eq!(
        events.recv().unwrap(),
        BackendEvent::Send("two".to_string(), vec![0x10])
    );
    assert_eq!(
        events.recv().unwrap(),
        BackendEvent::Send("one".to_string(), vec![0x20])
    );
    runtime.request_shutdown();
    owner.shutdown_and_join(Duration::from_secs(1));
    assert!(events.try_recv().is_err());
}

#[test]
fn refresh_and_dispatch_health_have_separate_provenance() {
    let (_focus_tx, focus_rx) = tokio::sync::watch::channel(WindowMetadata::default());
    let (backend, events) = backend([ready(1), ready(2), failed(3, "refresh")]);
    let failed_devices = backend.failed_devices.clone();
    let (runtime, owner) = start(Some(config(1, &["one"])), focus_rx, backend);
    finish_startup(&runtime, &events, 1);

    failed_devices.lock().unwrap().push("one".to_string());
    let result =
        block_on(runtime.test_action(action(1), vec![device("one"), device("two")])).unwrap();
    assert_eq!(result.sent, 1);
    assert_eq!(result.failures.len(), 1);
    assert_eq!(
        events.recv().unwrap(),
        BackendEvent::Send("one".to_string(), vec![1])
    );
    assert_eq!(
        events.recv().unwrap(),
        BackendEvent::Send("two".to_string(), vec![1])
    );
    assert_eq!(runtime.status().phase, RuntimePhase::Degraded);
    let empty = block_on(runtime.test_action(action(1), Vec::new())).unwrap();
    assert_eq!(
        empty,
        TestDispatchResult {
            sent: 0,
            failures: Vec::new()
        }
    );
    assert_eq!(runtime.status().phase, RuntimePhase::Degraded);

    assert_eq!(
        runtime.request_hid_refresh().unwrap(),
        HidRefreshRequestResult::Queued
    );
    assert_eq!(events.recv().unwrap(), BackendEvent::RefreshStarted);
    assert_eq!(events.recv().unwrap(), BackendEvent::RefreshFinished(2));
    wait_for_revision(&runtime, 2);
    assert_eq!(runtime.status().phase, RuntimePhase::Degraded);

    failed_devices.lock().unwrap().clear();
    block_on(runtime.test_action(action(1), vec![device("one")])).unwrap();
    assert_eq!(
        events.recv().unwrap(),
        BackendEvent::Send("one".to_string(), vec![1])
    );
    wait_for_phase(&runtime, RuntimePhase::Active);

    assert_eq!(
        runtime.request_hid_refresh().unwrap(),
        HidRefreshRequestResult::Queued
    );
    assert_eq!(events.recv().unwrap(), BackendEvent::RefreshStarted);
    assert_eq!(events.recv().unwrap(), BackendEvent::RefreshFinished(3));
    wait_for_revision(&runtime, 3);
    wait_for_phase(&runtime, RuntimePhase::Degraded);
    block_on(runtime.test_action(action(1), vec![device("one")])).unwrap();
    assert_eq!(
        events.recv().unwrap(),
        BackendEvent::Send("one".to_string(), vec![1])
    );
    assert_eq!(runtime.status().phase, RuntimePhase::Degraded);
    owner.shutdown_and_join(Duration::from_secs(1));
}

#[test]
fn send_publishes_implicit_refresh_outcomes() {
    let (_focus_tx, focus_rx) = tokio::sync::watch::channel(WindowMetadata::default());
    let (mut backend, events) = backend([ready(1)]);
    backend
        .send_inventories
        .extend([failed(2, "implicit failure"), ready(3)]);
    let (runtime, owner) = start(Some(config(1, &["one"])), focus_rx, backend);
    finish_startup(&runtime, &events, 1);
    let mut inventory = runtime.subscribe_hid_inventory();

    block_on(runtime.test_action(action(1), vec![device("one")])).unwrap();
    block_on(async { inventory.changed().await.unwrap() });
    assert_eq!(inventory.borrow().revision, 2);
    assert!(matches!(
        inventory.borrow().refresh_state,
        HidRefreshState::Failed { .. }
    ));
    assert_eq!(runtime.status().phase, RuntimePhase::Degraded);

    block_on(runtime.test_action(action(1), vec![device("one")])).unwrap();
    block_on(async { inventory.changed().await.unwrap() });
    assert_eq!(inventory.borrow().revision, 3);
    assert_eq!(inventory.borrow().refresh_state, HidRefreshState::Ready);
    wait_for_phase(&runtime, RuntimePhase::Active);
    owner.shutdown_and_join(Duration::from_secs(1));
}

#[test]
fn missing_focus_source_or_configuration_is_unavailable() {
    let (_focus_tx, focus_rx) = tokio::sync::watch::channel(WindowMetadata::default());
    let (backend, events) = backend([ready(1)]);
    let (runtime, owner) = AutomationRuntime::start(
        None,
        focus_rx,
        FocusSourceState::Unavailable("hook failed".to_string()),
        backend,
    )
    .unwrap();

    finish_startup(&runtime, &events, 1);
    assert_eq!(runtime.status().phase, RuntimePhase::Unavailable);
    assert!(runtime.status().detail.unwrap().contains("hook failed"));
    runtime.request_shutdown();
    runtime.request_shutdown();
    owner.shutdown_and_join(Duration::from_secs(1));
    assert_eq!(runtime.status().phase, RuntimePhase::Stopped);
}

#[test]
fn worker_panic_closes_admission_and_clears_pending_refresh() {
    let (_focus_tx, focus_rx) = tokio::sync::watch::channel(WindowMetadata::default());
    let (mut backend, _events) = backend([]);
    backend.panic_on_refresh = true;
    let (runtime, owner) = start(Some(config(1, &["one"])), focus_rx, backend);

    wait_for_phase(&runtime, RuntimePhase::Unavailable);
    assert!(runtime.request_hid_refresh().is_err());
    assert!(block_on(runtime.test_action(action(1), vec![device("one")])).is_err());
    owner.shutdown_and_join(Duration::from_secs(1));
}
