use std::{
    collections::VecDeque,
    future::Future,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

use super::{
    AutomationRuntime, FocusGenerationProgress, FocusSourceState, HidRefreshRequestResult,
    ORDINARY_COMMAND_CAPACITY, RuntimeOwner, RuntimePhase, RuntimeRequestError, TestDispatchResult,
};
use crate::{
    config::{
        ActiveConfig, Automation, AutomationCase, Config, Device, SendAction, TextCondition,
        ValidationError, WindowMatcher,
    },
    focused_window::{FocusedWindow, ForegroundObservation},
    hid::{HidBackend, HidError, HidInventory, HidRefreshState},
};

fn replace_config_for_test(
    runtime: &AutomationRuntime,
    config: Config,
) -> std::result::Result<(), Vec<ValidationError>> {
    let active = Arc::new(ActiveConfig::compile(&config)?);
    runtime.replace_active_config(active);
    Ok(())
}

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
    send_gates: VecDeque<Option<Gate>>,
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
        if let Some(Some((started, release))) = self.send_gates.pop_front() {
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
            send_gates: VecDeque::new(),
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

fn routed_config(routes: &[(&str, u8)]) -> Config {
    Config {
        devices: vec![device("automatic")],
        automations: vec![Automation {
            id: "automation".to_string(),
            name: "Automation".to_string(),
            enabled: true,
            cases: routes
                .iter()
                .enumerate()
                .map(|(index, (title, report))| AutomationCase {
                    id: format!("case-{index}"),
                    name: format!("Case {index}"),
                    applications: vec![WindowMatcher {
                        id: format!("matcher-{index}"),
                        title: Some(TextCondition::equals(*title)),
                        ..WindowMatcher::default()
                    }],
                    actions: vec![SendAction {
                        id: format!("action-{index}"),
                        report: vec![*report],
                        device_ids: vec!["automatic".to_string()],
                        ..SendAction::default()
                    }],
                    ..AutomationCase::default()
                })
                .collect(),
            ..Automation::default()
        }],
        ..Config::default()
    }
}

fn focused(generation: u64, title: &str) -> ForegroundObservation {
    ForegroundObservation {
        generation,
        raw_hwnd: generation as isize,
        window: FocusedWindow {
            title: Some(title.to_string()),
            ..FocusedWindow::default()
        },
    }
}

fn start(
    config: Option<Config>,
    focus_rx: tokio::sync::watch::Receiver<ForegroundObservation>,
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

fn focus_progress(runtime: &AutomationRuntime) -> FocusGenerationProgress {
    let progress = runtime.focus_progress_snapshot();
    for generation in [
        progress.latest_started,
        progress.latest_handled,
        progress.latest_cancelled,
    ]
    .into_iter()
    .flatten()
    {
        assert!(
            generation <= progress.latest_observed,
            "runtime generation {generation} exceeds latest observed generation {}",
            progress.latest_observed
        );
    }
    progress
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
    let (_focus_tx, focus_rx) = tokio::sync::watch::channel(ForegroundObservation::default());
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
fn startup_refresh_completes_before_retained_initial_focus_runs() {
    let (_focus_tx, focus_rx) = tokio::sync::watch::channel(focused(1, "target"));
    let (backend, events) = backend([ready(1)]);
    let (runtime, owner) = start(Some(config(0x10, &["automatic"])), focus_rx, backend);

    assert_eq!(events.recv().unwrap(), BackendEvent::RefreshStarted);
    assert_eq!(events.recv().unwrap(), BackendEvent::RefreshFinished(1));
    assert_eq!(
        events.recv().unwrap(),
        BackendEvent::Send("automatic".to_string(), vec![0x10])
    );
    wait_until(|| focus_progress(&runtime).latest_handled == Some(1));
    owner.shutdown_and_join(Duration::from_secs(1));
}

#[test]
fn shutdown_completed_before_initialization_claim_cancels_startup_refresh() {
    let (_focus_tx, focus_rx) = tokio::sync::watch::channel(focused(1, "target"));
    let (backend, events) = backend([ready(1)]);
    let (runtime, owner, claim_reached, release_claim) =
        AutomationRuntime::start_with_initialization_claim_gate(
            Some(config(0x10, &["automatic"])),
            focus_rx,
            FocusSourceState::Available,
            backend,
        )
        .unwrap();

    claim_reached.recv().unwrap();
    runtime.request_shutdown();
    release_claim.send(()).unwrap();
    owner.shutdown_and_join(Duration::from_secs(1));

    assert_eq!(runtime.status().phase, RuntimePhase::Stopped);
    assert_eq!(runtime.hid_inventory().revision, 0);
    assert_eq!(
        focus_progress(&runtime),
        FocusGenerationProgress {
            latest_observed: 1,
            latest_started: None,
            latest_handled: None,
            latest_cancelled: Some(1),
        }
    );
    assert!(events.try_recv().is_err());
}

#[test]
fn shutdown_after_initialization_claim_waits_for_atomic_startup_refresh() {
    let (_focus_tx, focus_rx) = tokio::sync::watch::channel(ForegroundObservation::default());
    let (mut backend, events) = backend([ready(1)]);
    let (refresh_started_tx, refresh_started) = mpsc::channel();
    let (refresh_release_tx, refresh_release) = mpsc::channel();
    backend
        .refresh_gates
        .push_back(Some((refresh_started_tx, refresh_release)));
    let (runtime, owner) = start(Some(config(0x10, &["automatic"])), focus_rx, backend);

    refresh_started.recv().unwrap();
    assert_eq!(events.recv().unwrap(), BackendEvent::RefreshStarted);
    runtime.request_shutdown();
    assert_eq!(runtime.status().phase, RuntimePhase::Stopping);
    assert!(events.try_recv().is_err());

    refresh_release_tx.send(()).unwrap();
    assert_eq!(events.recv().unwrap(), BackendEvent::RefreshFinished(1));
    owner.shutdown_and_join(Duration::from_secs(1));
    assert_eq!(runtime.status().phase, RuntimePhase::Stopped);
}

#[test]
fn shutdown_completed_before_claim_cancels_the_provisional_focus() {
    let (focus_tx, focus_rx) = tokio::sync::watch::channel(ForegroundObservation::default());
    let (backend, events) = backend([ready(1)]);
    let (runtime, owner) = start(Some(config(0x10, &["automatic"])), focus_rx, backend);
    finish_startup(&runtime, &events, 1);
    let (claim_reached, release_claim) = runtime.gate_next_focus_boundary_claim();

    focus_tx.send_replace(focused(1, "target"));
    claim_reached.recv().unwrap();
    runtime.request_shutdown();
    release_claim.send(()).unwrap();
    owner.shutdown_and_join(Duration::from_secs(1));

    assert_eq!(
        focus_progress(&runtime),
        FocusGenerationProgress {
            latest_observed: 1,
            latest_started: None,
            latest_handled: None,
            latest_cancelled: Some(1),
        }
    );
    assert!(events.try_recv().is_err());
}

#[test]
fn newer_focus_completed_before_claim_replaces_the_provisional_generation() {
    let (focus_tx, focus_rx) = tokio::sync::watch::channel(ForegroundObservation::default());
    let (backend, events) = backend([ready(1)]);
    let (runtime, owner) = start(
        Some(routed_config(&[("A", 0x0a), ("B", 0x0b)])),
        focus_rx,
        backend,
    );
    finish_startup(&runtime, &events, 1);
    let (claim_reached, release_claim) = runtime.gate_next_focus_boundary_claim();

    focus_tx.send_replace(focused(1, "A"));
    claim_reached.recv().unwrap();
    focus_tx.send_replace(focused(2, "B"));
    release_claim.send(()).unwrap();

    assert_eq!(
        events.recv().unwrap(),
        BackendEvent::Send("automatic".to_string(), vec![0x0b])
    );
    wait_until(|| focus_progress(&runtime).latest_handled == Some(2));
    assert_eq!(
        focus_progress(&runtime),
        FocusGenerationProgress {
            latest_observed: 2,
            latest_started: Some(2),
            latest_handled: Some(2),
            latest_cancelled: None,
        }
    );
    owner.shutdown_and_join(Duration::from_secs(1));
    assert!(events.try_recv().is_err());
}

#[test]
fn config_replacement_completed_before_claim_supplies_the_focus_snapshot() {
    let (focus_tx, focus_rx) = tokio::sync::watch::channel(ForegroundObservation::default());
    let (backend, events) = backend([ready(1)]);
    let (runtime, owner) = start(Some(config(0x10, &["automatic"])), focus_rx, backend);
    finish_startup(&runtime, &events, 1);
    let (claim_reached, release_claim) = runtime.gate_next_focus_boundary_claim();

    focus_tx.send_replace(focused(1, "target"));
    claim_reached.recv().unwrap();
    replace_config_for_test(&runtime, config(0x20, &["automatic"])).unwrap();
    release_claim.send(()).unwrap();

    assert_eq!(
        events.recv().unwrap(),
        BackendEvent::Send("automatic".to_string(), vec![0x20])
    );
    owner.shutdown_and_join(Duration::from_secs(1));
}

#[test]
fn focus_precedes_an_admitted_command_at_the_same_worker_boundary() {
    let (focus_tx, focus_rx) = tokio::sync::watch::channel(ForegroundObservation::default());
    let (mut backend, events) = backend([ready(1)]);
    let (refresh_started_tx, refresh_started) = mpsc::channel();
    let (refresh_release_tx, refresh_release) = mpsc::channel();
    backend
        .refresh_gates
        .push_back(Some((refresh_started_tx, refresh_release)));
    let (runtime, owner) = start(Some(config(0x10, &["automatic"])), focus_rx, backend);

    refresh_started.recv().unwrap();
    assert_eq!(events.recv().unwrap(), BackendEvent::RefreshStarted);
    focus_tx.send_replace(focused(1, "target"));

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
            BackendEvent::Send("automatic".to_string(), vec![0x10])
        );
        assert_eq!(
            events.recv().unwrap(),
            BackendEvent::Send("manual".to_string(), vec![0x20])
        );
        assert_eq!(test.await.unwrap().sent, 1);
    });

    owner.shutdown_and_join(Duration::from_secs(1));
}

#[test]
fn pending_focus_coalesces_to_the_newest_observation_before_start() {
    let (focus_tx, focus_rx) = tokio::sync::watch::channel(ForegroundObservation::default());
    let (mut backend, events) = backend([ready(1)]);
    let (refresh_started_tx, refresh_started) = mpsc::channel();
    let (refresh_release_tx, refresh_release) = mpsc::channel();
    backend
        .refresh_gates
        .push_back(Some((refresh_started_tx, refresh_release)));
    let (runtime, owner) = start(
        Some(routed_config(&[("A", 0x0a), ("B", 0x0b)])),
        focus_rx,
        backend,
    );

    refresh_started.recv().unwrap();
    assert_eq!(events.recv().unwrap(), BackendEvent::RefreshStarted);
    focus_tx.send_replace(focused(1, "A"));
    focus_tx.send_replace(focused(2, "B"));
    refresh_release_tx.send(()).unwrap();

    assert_eq!(events.recv().unwrap(), BackendEvent::RefreshFinished(1));
    assert_eq!(
        events.recv().unwrap(),
        BackendEvent::Send("automatic".to_string(), vec![0x0b])
    );
    wait_for_phase(&runtime, RuntimePhase::Active);
    runtime.request_shutdown();
    owner.shutdown_and_join(Duration::from_secs(1));
    assert!(events.try_recv().is_err());
}

#[test]
fn replacement_before_a_focus_boundary_supplies_that_batch_snapshot() {
    let (focus_tx, focus_rx) = tokio::sync::watch::channel(ForegroundObservation::default());
    let (mut backend, events) = backend([ready(1)]);
    let (refresh_started_tx, refresh_started) = mpsc::channel();
    let (refresh_release_tx, refresh_release) = mpsc::channel();
    backend
        .refresh_gates
        .push_back(Some((refresh_started_tx, refresh_release)));
    let (runtime, owner) = start(Some(config(0x10, &["automatic"])), focus_rx, backend);

    refresh_started.recv().unwrap();
    assert_eq!(events.recv().unwrap(), BackendEvent::RefreshStarted);
    focus_tx.send_replace(focused(1, "target"));
    replace_config_for_test(&runtime, config(0x20, &["automatic"])).unwrap();
    refresh_release_tx.send(()).unwrap();

    assert_eq!(events.recv().unwrap(), BackendEvent::RefreshFinished(1));
    assert_eq!(
        events.recv().unwrap(),
        BackendEvent::Send("automatic".to_string(), vec![0x20])
    );
    owner.shutdown_and_join(Duration::from_secs(1));
}

#[test]
fn focus_precedes_queued_test_and_explicit_refresh_after_an_atomic_batch() {
    let (focus_tx, focus_rx) = tokio::sync::watch::channel(ForegroundObservation::default());
    let (mut backend, events) = backend([ready(1), ready(2)]);
    let (send_started_tx, send_started) = mpsc::channel();
    let (send_release_tx, send_release) = mpsc::channel();
    backend
        .send_gates
        .push_back(Some((send_started_tx, send_release)));
    let (runtime, owner) = start(Some(config(0x10, &["automatic"])), focus_rx, backend);
    finish_startup(&runtime, &events, 1);

    let running = runtime
        .admit_test_action(action(0x01), vec![device("running")])
        .unwrap();
    send_started.recv().unwrap();
    assert_eq!(
        events.recv().unwrap(),
        BackendEvent::Send("running".to_string(), vec![0x01])
    );
    assert_eq!(
        runtime.request_hid_refresh().unwrap(),
        HidRefreshRequestResult::Queued
    );
    let queued = runtime
        .admit_test_action(action(0x02), vec![device("queued")])
        .unwrap();
    focus_tx.send_replace(focused(1, "target"));
    send_release_tx.send(()).unwrap();

    assert_eq!(
        events.recv().unwrap(),
        BackendEvent::Send("automatic".to_string(), vec![0x10])
    );
    assert_eq!(events.recv().unwrap(), BackendEvent::RefreshStarted);
    assert_eq!(events.recv().unwrap(), BackendEvent::RefreshFinished(2));
    assert_eq!(
        events.recv().unwrap(),
        BackendEvent::Send("queued".to_string(), vec![0x02])
    );
    assert_eq!(block_on(running).unwrap().unwrap().sent, 1);
    assert_eq!(block_on(queued).unwrap().unwrap().sent, 1);
    owner.shutdown_and_join(Duration::from_secs(1));
}

#[test]
fn accepted_test_may_starve_during_focus_churn_then_recovers() {
    let (focus_tx, focus_rx) = tokio::sync::watch::channel(ForegroundObservation::default());
    let (mut backend, events) = backend([ready(1)]);
    let mut releases = Vec::new();
    let mut starts = Vec::new();
    for _ in 0..3 {
        let (started_tx, started) = mpsc::channel();
        let (release_tx, release) = mpsc::channel();
        backend.send_gates.push_back(Some((started_tx, release)));
        starts.push(started);
        releases.push(release_tx);
    }
    let (runtime, owner) = start(Some(config(0x10, &["automatic"])), focus_rx, backend);
    finish_startup(&runtime, &events, 1);

    focus_tx.send_replace(focused(1, "target A"));
    starts.remove(0).recv().unwrap();
    assert_eq!(
        events.recv().unwrap(),
        BackendEvent::Send("automatic".to_string(), vec![0x10])
    );
    let starved = runtime
        .admit_test_action(action(0x20), vec![device("manual")])
        .unwrap();

    for generation in 2..=3 {
        focus_tx.send_replace(focused(generation, &format!("target {generation}")));
        releases.remove(0).send(()).unwrap();
        starts.remove(0).recv().unwrap();
        assert_eq!(
            events.recv().unwrap(),
            BackendEvent::Send("automatic".to_string(), vec![0x10])
        );
    }
    releases.remove(0).send(()).unwrap();

    assert_eq!(
        events.recv().unwrap(),
        BackendEvent::Send("manual".to_string(), vec![0x20])
    );
    assert_eq!(block_on(starved).unwrap().unwrap().sent, 1);
    owner.shutdown_and_join(Duration::from_secs(1));
}

#[test]
fn no_action_focus_is_handled_without_retriggering_after_replacement() {
    let (focus_tx, focus_rx) = tokio::sync::watch::channel(ForegroundObservation::default());
    let (backend, events) = backend([ready(1), ready(2)]);
    let (runtime, owner) = start(Some(config(0x10, &["automatic"])), focus_rx, backend);
    finish_startup(&runtime, &events, 1);

    focus_tx.send_replace(focused(1, "unmatched"));
    let marker = runtime
        .admit_test_action(action(0x30), vec![device("marker")])
        .unwrap();
    assert_eq!(
        events.recv().unwrap(),
        BackendEvent::Send("marker".to_string(), vec![0x30])
    );
    assert_eq!(block_on(marker).unwrap().unwrap().sent, 1);
    assert_eq!(focus_progress(&runtime).latest_handled, Some(1));

    replace_config_for_test(&runtime, routed_config(&[("unmatched", 0x20)])).unwrap();
    assert_eq!(
        runtime.request_hid_refresh().unwrap(),
        HidRefreshRequestResult::Queued
    );
    assert_eq!(events.recv().unwrap(), BackendEvent::RefreshStarted);
    assert_eq!(events.recv().unwrap(), BackendEvent::RefreshFinished(2));
    focus_tx.send_replace(focused(2, "unmatched"));
    assert_eq!(
        events.recv().unwrap(),
        BackendEvent::Send("automatic".to_string(), vec![0x20])
    );
    owner.shutdown_and_join(Duration::from_secs(1));
}

#[test]
fn invalid_replacement_is_rejected_without_filtering_or_replacing_routes() {
    let (focus_tx, focus_rx) = tokio::sync::watch::channel(ForegroundObservation::default());
    let (backend, events) = backend([ready(1)]);
    let (runtime, owner) = start(Some(config(0x10, &["automatic"])), focus_rx, backend);
    finish_startup(&runtime, &events, 1);

    let mut invalid = config(0x20, &["automatic"]);
    invalid.automations[0].cases[0].actions[0].device_ids = vec!["missing".to_string()];
    assert!(replace_config_for_test(&runtime, invalid).is_err());
    focus_tx.send_replace(focused(1, "target"));
    assert_eq!(
        events.recv().unwrap(),
        BackendEvent::Send("automatic".to_string(), vec![0x10])
    );
    owner.shutdown_and_join(Duration::from_secs(1));
}

#[test]
fn startup_failure_stays_degraded_after_test_and_explicit_refresh_recovers() {
    let (_focus_tx, focus_rx) = tokio::sync::watch::channel(ForegroundObservation::default());
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
    let (_focus_tx, focus_rx) = tokio::sync::watch::channel(ForegroundObservation::default());
    let (mut backend, events) = backend([ready(1), ready(2), ready(3)]);
    let (send_started_tx, send_started) = mpsc::channel();
    let (send_release_tx, send_release) = mpsc::channel();
    backend
        .send_gates
        .push_back(Some((send_started_tx, send_release)));
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
fn admitted_test_refresh_and_test_execute_fifo_without_interleaving() {
    let (_focus_tx, focus_rx) = tokio::sync::watch::channel(ForegroundObservation::default());
    let (mut backend, events) = backend([ready(1), ready(2)]);
    let (send_started_tx, send_started) = mpsc::channel();
    let (send_release_tx, send_release) = mpsc::channel();
    backend
        .send_gates
        .push_back(Some((send_started_tx, send_release)));
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

        runtime.request_shutdown();
        runtime.request_shutdown();
        assert_eq!(
            runtime.request_hid_refresh(),
            Err(RuntimeRequestError::Unavailable)
        );
        assert_eq!(
            runtime.test_action(action(3), vec![device("late")]).await,
            Err(RuntimeRequestError::Unavailable)
        );
    });

    owner.shutdown_and_join(Duration::from_secs(1));
    assert_eq!(runtime.status().phase, RuntimePhase::Stopped);
    assert!(events.try_recv().is_err());
}

#[test]
fn saturated_queue_rejects_ordinary_work_but_shutdown_cancels_every_queued_test() {
    let (focus_tx, focus_rx) = tokio::sync::watch::channel(ForegroundObservation::default());
    let (mut backend, events) = backend([ready(1)]);
    let (send_started_tx, send_started) = mpsc::channel();
    let (send_release_tx, send_release) = mpsc::channel();
    backend
        .send_gates
        .push_back(Some((send_started_tx, send_release)));
    let (runtime, owner) = start(Some(config(1, &["running"])), focus_rx, backend);
    finish_startup(&runtime, &events, 1);

    let running = runtime
        .admit_test_action(action(1), vec![device("running")])
        .unwrap();
    send_started.recv().unwrap();
    assert_eq!(
        events.recv().unwrap(),
        BackendEvent::Send("running".to_string(), vec![1])
    );

    let queued = (0..ORDINARY_COMMAND_CAPACITY)
        .map(|index| {
            runtime
                .admit_test_action(action(2), vec![device(&format!("queued-{index}"))])
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        block_on(runtime.test_action(action(3), vec![device("full")])),
        Err(RuntimeRequestError::Busy)
    );
    assert_eq!(
        runtime.request_hid_refresh(),
        Err(RuntimeRequestError::Busy)
    );

    focus_tx.send_replace(focused(1, "target"));
    runtime.request_shutdown();
    runtime.request_shutdown();
    assert_eq!(runtime.status().phase, RuntimePhase::Stopping);
    send_release_tx.send(()).unwrap();

    assert_eq!(block_on(running).unwrap().unwrap().sent, 1);
    for response in queued {
        assert_eq!(
            block_on(response).unwrap(),
            Err(RuntimeRequestError::Cancelled)
        );
    }
    owner.shutdown_and_join(Duration::from_secs(1));

    assert_eq!(runtime.status().phase, RuntimePhase::Stopped);
    assert!(events.try_recv().is_err());
}

#[test]
fn status_publication_does_not_regress_after_shutdown_starts() {
    let (_focus_tx, focus_rx) = tokio::sync::watch::channel(ForegroundObservation::default());
    let (mut backend, events) = backend([ready(1)]);
    let (send_started_tx, send_started) = mpsc::channel();
    let (send_release_tx, send_release) = mpsc::channel();
    backend
        .send_gates
        .push_back(Some((send_started_tx, send_release)));
    let (runtime, owner) = start(Some(config(1, &["running"])), focus_rx, backend);
    finish_startup(&runtime, &events, 1);

    let running = runtime
        .admit_test_action(action(1), vec![device("running")])
        .unwrap();
    send_started.recv().unwrap();
    assert!(matches!(events.recv().unwrap(), BackendEvent::Send(..)));
    runtime.request_shutdown();
    assert_eq!(runtime.status().phase, RuntimePhase::Stopping);

    send_release_tx.send(()).unwrap();
    assert_eq!(block_on(running).unwrap().unwrap().sent, 1);
    owner.shutdown_and_join(Duration::from_secs(1));

    let history = runtime.status_history();
    let stopping = history
        .iter()
        .position(|status| status.phase == RuntimePhase::Stopping)
        .unwrap();
    assert!(
        history[stopping..]
            .iter()
            .all(|status| matches!(status.phase, RuntimePhase::Stopping | RuntimePhase::Stopped))
    );
}

#[test]
fn automatic_and_manual_batches_do_not_interleave() {
    let (focus_tx, focus_rx) = tokio::sync::watch::channel(ForegroundObservation::default());
    let (mut backend, events) = backend([ready(1)]);
    let (send_started_tx, send_started) = mpsc::channel();
    let (send_release_tx, send_release) = mpsc::channel();
    backend
        .send_gates
        .push_back(Some((send_started_tx, send_release)));
    let (runtime, owner) = start(Some(config(0x10, &["one", "two"])), focus_rx, backend);
    finish_startup(&runtime, &events, 1);

    focus_tx.send_replace(focused(1, "target"));
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
    let (focus_tx, focus_rx) = tokio::sync::watch::channel(ForegroundObservation::default());
    let (mut backend, events) = backend([ready(1)]);
    let (send_started_tx, send_started) = mpsc::channel();
    let (send_release_tx, send_release) = mpsc::channel();
    backend
        .send_gates
        .push_back(Some((send_started_tx, send_release)));
    let (runtime, owner) = start(Some(config(0x10, &["one", "two"])), focus_rx, backend);
    finish_startup(&runtime, &events, 1);

    focus_tx.send_replace(focused(1, "target first"));
    send_started.recv().unwrap();
    assert_eq!(
        events.recv().unwrap(),
        BackendEvent::Send("one".to_string(), vec![0x10])
    );
    assert_eq!(
        focus_progress(&runtime),
        FocusGenerationProgress {
            latest_observed: 1,
            latest_started: Some(1),
            latest_handled: None,
            latest_cancelled: None,
        }
    );
    replace_config_for_test(&runtime, config(0x20, &["one"])).unwrap();
    focus_tx.send_replace(focused(2, "not matching"));
    focus_tx.send_replace(focused(3, "target latest"));
    assert_eq!(
        focus_progress(&runtime),
        FocusGenerationProgress {
            latest_observed: 3,
            latest_started: Some(1),
            latest_handled: None,
            latest_cancelled: None,
        }
    );
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
    let (_focus_tx, focus_rx) = tokio::sync::watch::channel(ForegroundObservation::default());
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
    let (_focus_tx, focus_rx) = tokio::sync::watch::channel(ForegroundObservation::default());
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
    let (_focus_tx, focus_rx) = tokio::sync::watch::channel(ForegroundObservation::default());
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
    let (_focus_tx, focus_rx) = tokio::sync::watch::channel(ForegroundObservation::default());
    let (mut backend, _events) = backend([]);
    backend.panic_on_refresh = true;
    let (runtime, owner) = start(Some(config(1, &["one"])), focus_rx, backend);

    wait_for_phase(&runtime, RuntimePhase::Unavailable);
    assert!(runtime.request_hid_refresh().is_err());
    assert!(block_on(runtime.test_action(action(1), vec![device("one")])).is_err());
    owner.shutdown_and_join(Duration::from_secs(1));
}
