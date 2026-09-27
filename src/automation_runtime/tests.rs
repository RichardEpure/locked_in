use std::{
    collections::VecDeque,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex, mpsc},
    task::{Context, Poll, Waker},
    thread,
    time::{Duration, Instant},
};

use super::{
    AutomationRuntime, EventSourceState, FocusInput, HidRefreshRequestResult,
    ORDINARY_COMMAND_CAPACITY, RuntimeInputs, RuntimeOwner, RuntimePhase, RuntimeRequestError,
    RuntimeStatus, TestDispatchResult,
    inputs::{FocusGenerationProgress, FocusProgress},
};
use crate::{
    config::{
        Automation, AutomationCase, Device, EditableConfig, PublishedConfig, SendAction,
        TextCondition, ValidationError, WindowMatcher,
    },
    focused_window::{FocusedWindow, ForegroundObservation},
    hid::{HidBackend, HidError, HidInventory, HidRefreshState},
};

pub(super) struct ClaimGate {
    started: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
}

#[derive(Clone)]
struct TestRuntime {
    runtime: AutomationRuntime,
    publications: Option<tokio::sync::watch::Sender<Arc<PublishedConfig>>>,
}

impl std::ops::Deref for TestRuntime {
    type Target = AutomationRuntime;
    fn deref(&self) -> &Self::Target {
        &self.runtime
    }
}

fn publication_channel(
    config: Option<EditableConfig>,
) -> anyhow::Result<Option<tokio::sync::watch::Sender<Arc<PublishedConfig>>>> {
    config
        .map(|config| {
            PublishedConfig::prepare_for_test(config, 1)
                .map(|publication| tokio::sync::watch::channel(publication).0)
        })
        .transpose()
        .map_err(format_compilation_errors)
}

impl AutomationRuntime {
    fn start_with_config(
        initial_config: Option<EditableConfig>,
        focused_window: FocusInput,
        backend: impl HidBackend,
    ) -> anyhow::Result<(TestRuntime, RuntimeOwner)> {
        let publications = publication_channel(initial_config)?;
        let receiver = publications
            .as_ref()
            .map(tokio::sync::watch::Sender::subscribe);
        let (runtime, owner) = Self::start(receiver, RuntimeInputs { focused_window }, backend)?;
        Ok((
            TestRuntime {
                runtime,
                publications,
            },
            owner,
        ))
    }

    fn start_with_initialization_claim_gate(
        initial_config: Option<EditableConfig>,
        focused_window: FocusInput,
        backend: impl HidBackend,
    ) -> anyhow::Result<(
        TestRuntime,
        RuntimeOwner,
        mpsc::Receiver<()>,
        mpsc::Sender<()>,
    )> {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let publications = publication_channel(initial_config)?;
        let receiver = publications
            .as_ref()
            .map(tokio::sync::watch::Sender::subscribe);
        let (runtime, owner) = Self::start_inner(
            receiver,
            RuntimeInputs { focused_window },
            backend,
            Some(ClaimGate {
                started: started_tx,
                release: release_rx,
            }),
        )?;
        Ok((
            TestRuntime {
                runtime,
                publications,
            },
            owner,
            started_rx,
            release_tx,
        ))
    }

    fn status(&self) -> RuntimeStatus {
        self.subscribe_status().borrow().clone()
    }

    fn hid_inventory(&self) -> Arc<HidInventory> {
        self.shared.hid_inventory.borrow().clone()
    }

    fn gate_next_boundary_claim(&self) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        *self
            .shared
            .boundary_claim_gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(ClaimGate {
            started: started_tx,
            release: release_rx,
        });
        (started_rx, release_tx)
    }

    fn status_history(&self) -> Vec<RuntimeStatus> {
        self.shared.health.history()
    }
}

impl super::Shared {
    pub(super) fn wait_before_initialization_claim(&self) {
        let gate = self
            .initialization_claim_gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(gate) = gate {
            gate.started.send(()).unwrap();
            gate.release.recv().unwrap();
        }
    }

    pub(super) fn wait_before_boundary_claim(&self) {
        let gate = self
            .boundary_claim_gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(gate) = gate {
            gate.started.send(()).unwrap();
            gate.release.recv().unwrap();
        }
    }
}

fn format_compilation_errors(errors: Vec<ValidationError>) -> anyhow::Error {
    let details = errors
        .iter()
        .map(|error| format!("{}: {}", error.path, error.message))
        .collect::<Vec<_>>()
        .join("; ");
    anyhow::anyhow!("configuration could not be activated: {details}")
}

fn replace_config_for_test(
    runtime: &TestRuntime,
    config: EditableConfig,
) -> std::result::Result<(), Vec<ValidationError>> {
    let sender = runtime.publications.as_ref().unwrap();
    let revision = sender.borrow().revision() + 1;
    let published = PublishedConfig::prepare_for_test(config, revision)?;
    sender.send_replace(published);
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BackendEvent {
    RefreshStarted,
    RefreshFinished(u64),
    Send(String, Vec<u8>),
}

type Gate = (mpsc::Sender<()>, mpsc::Receiver<()>);

fn gate_next_io(gates: &mut VecDeque<Option<Gate>>) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
    let (started_tx, started) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    gates.push_back(Some((started_tx, release_rx)));
    (started, release)
}

struct RecordingBackend {
    events: mpsc::Sender<BackendEvent>,
    inventory: HidInventory,
    refresh_results: VecDeque<HidInventory>,
    refresh_gates: VecDeque<Option<Gate>>,
    failed_devices: Arc<Mutex<Vec<String>>>,
    sent_devices: Arc<Mutex<Vec<Device>>>,
    send_gates: VecDeque<Option<Gate>>,
    send_inventories: VecDeque<HidInventory>,
    panic_on_refresh: bool,
    panic_on_send: bool,
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
        self.sent_devices.lock().unwrap().push(device.clone());
        self.events
            .send(BackendEvent::Send(device.id.clone(), report.to_vec()))
            .unwrap();
        if let Some(Some((started, release))) = self.send_gates.pop_front() {
            started.send(()).unwrap();
            release.recv().unwrap();
        }
        assert!(!self.panic_on_send, "configured send panic");
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
            sent_devices: Arc::new(Mutex::new(Vec::new())),
            send_gates: VecDeque::new(),
            send_inventories: VecDeque::new(),
            panic_on_refresh: false,
            panic_on_send: false,
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

fn config(report: u8, device_ids: &[&str]) -> EditableConfig {
    let devices = device_ids.iter().map(|id| device(id)).collect::<Vec<_>>();
    EditableConfig {
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
        ..EditableConfig::default()
    }
}

fn routed_config(routes: &[(&str, u8)]) -> EditableConfig {
    EditableConfig {
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
        ..EditableConfig::default()
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
    config: Option<EditableConfig>,
    focus_rx: tokio::sync::watch::Receiver<ForegroundObservation>,
    backend: RecordingBackend,
) -> (TestRuntime, RuntimeOwner) {
    AutomationRuntime::start_with_config(
        config,
        FocusInput::new(focus_rx, EventSourceState::Available),
        backend,
    )
    .unwrap()
}

fn start_with_progress(
    config: Option<EditableConfig>,
    focus_rx: tokio::sync::watch::Receiver<ForegroundObservation>,
    backend: RecordingBackend,
) -> (TestRuntime, RuntimeOwner, FocusProgress) {
    let focus = FocusInput::new(focus_rx, EventSourceState::Available);
    let progress = focus.progress();
    let (runtime, owner) = AutomationRuntime::start_with_config(config, focus, backend).unwrap();
    (runtime, owner, progress)
}

fn block_on<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
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

fn focus_progress(handle: &FocusProgress) -> FocusGenerationProgress {
    let progress = handle.snapshot();
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
    let (started, release_tx) = gate_next_io(&mut backend.refresh_gates);
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
    let (_runtime, owner, progress) =
        start_with_progress(Some(config(0x10, &["automatic"])), focus_rx, backend);

    assert_eq!(events.recv().unwrap(), BackendEvent::RefreshStarted);
    assert_eq!(events.recv().unwrap(), BackendEvent::RefreshFinished(1));
    assert_eq!(
        events.recv().unwrap(),
        BackendEvent::Send("automatic".to_string(), vec![0x10])
    );
    wait_until(|| focus_progress(&progress).latest_handled == Some(1));
    owner.shutdown_and_join(Duration::from_secs(1));
}

#[test]
fn shutdown_completed_before_initialization_claim_cancels_startup_refresh() {
    let (_focus_tx, focus_rx) = tokio::sync::watch::channel(focused(1, "target"));
    let (backend, events) = backend([ready(1)]);
    let focus = FocusInput::new(focus_rx, EventSourceState::Available);
    let progress = focus.progress();
    let (runtime, owner, claim_reached, release_claim) =
        AutomationRuntime::start_with_initialization_claim_gate(
            Some(config(0x10, &["automatic"])),
            focus,
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
        focus_progress(&progress),
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
    let (refresh_started, refresh_release_tx) = gate_next_io(&mut backend.refresh_gates);
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
    let (runtime, owner, progress) =
        start_with_progress(Some(config(0x10, &["automatic"])), focus_rx, backend);
    finish_startup(&runtime, &events, 1);
    let (claim_reached, release_claim) = runtime.gate_next_boundary_claim();

    focus_tx.send_replace(focused(1, "target"));
    claim_reached.recv().unwrap();
    runtime.request_shutdown();
    release_claim.send(()).unwrap();
    owner.shutdown_and_join(Duration::from_secs(1));

    assert_eq!(
        focus_progress(&progress),
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
    let (runtime, owner, progress) = start_with_progress(
        Some(routed_config(&[("A", 0x0a), ("B", 0x0b)])),
        focus_rx,
        backend,
    );
    finish_startup(&runtime, &events, 1);
    let (claim_reached, release_claim) = runtime.gate_next_boundary_claim();

    focus_tx.send_replace(focused(1, "A"));
    claim_reached.recv().unwrap();
    focus_tx.send_replace(focused(2, "B"));
    release_claim.send(()).unwrap();

    assert_eq!(
        events.recv().unwrap(),
        BackendEvent::Send("automatic".to_string(), vec![0x0b])
    );
    wait_until(|| focus_progress(&progress).latest_handled == Some(2));
    assert_eq!(
        focus_progress(&progress),
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
    let (claim_reached, release_claim) = runtime.gate_next_boundary_claim();

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
fn coordinator_publication_closes_startup_and_claim_gaps_and_survives_source_closure() {
    use crate::config::{
        ConfigCoordinator, ConfigStore, StartWithWindows, StartWithWindowsOutcome,
    };
    struct Startup;
    impl StartWithWindows for Startup {
        fn reconcile(&self, desired: bool) -> StartWithWindowsOutcome {
            StartWithWindowsOutcome::confirmed(desired)
        }
    }
    struct Directory(std::path::PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let directory = Directory(std::env::temp_dir().join(format!(
        "locked-in-runtime-publication-{}",
        std::process::id()
    )));
    std::fs::create_dir_all(&directory.0).unwrap();
    let store = Arc::new(ConfigStore::new(directory.0.join("config.toml")));
    store.save_for_test(&config(0x10, &["automatic"])).unwrap();
    let coordinator = ConfigCoordinator::initial_load(store, Arc::new(Startup)).unwrap();
    let publications = coordinator.subscribe();
    // Publication after subscribing but before starting the worker must be visible.
    coordinator.update(1, config(0x20, &["automatic"])).unwrap();
    let (focus_tx, focus_rx) = tokio::sync::watch::channel(focused(1, "target"));
    let (backend, events) = backend([ready(1)]);
    let (runtime, owner) = AutomationRuntime::start(
        Some(publications),
        RuntimeInputs {
            focused_window: FocusInput::new(focus_rx, EventSourceState::Available),
        },
        backend,
    )
    .unwrap();
    finish_startup(&runtime, &events, 1);
    assert_eq!(
        events.recv_timeout(Duration::from_secs(2)).unwrap(),
        BackendEvent::Send("automatic".into(), vec![0x20])
    );
    let (claim_reached, release_claim) = runtime.gate_next_boundary_claim();
    focus_tx.send_replace(focused(2, "target newer"));
    claim_reached.recv_timeout(Duration::from_secs(2)).unwrap();
    coordinator.update(2, config(0x30, &["automatic"])).unwrap();
    drop(coordinator);
    release_claim.send(()).unwrap();
    assert_eq!(
        events.recv_timeout(Duration::from_secs(2)).unwrap(),
        BackendEvent::Send("automatic".into(), vec![0x30])
    );
    focus_tx.send_replace(focused(3, "target after closure"));
    assert_eq!(
        events.recv_timeout(Duration::from_secs(2)).unwrap(),
        BackendEvent::Send("automatic".into(), vec![0x30])
    );
    owner.shutdown_and_join(Duration::from_secs(1));
    assert!(events.try_recv().is_err());
}

#[test]
fn newest_focus_arriving_after_command_staging_runs_before_fifo_commands() {
    let (focus_tx, focus_rx) = tokio::sync::watch::channel(ForegroundObservation::default());
    let (backend, events) = backend([ready(1)]);
    let (runtime, owner) = start(
        Some(routed_config(&[("A", 0x0a), ("B", 0x0b)])),
        focus_rx,
        backend,
    );
    finish_startup(&runtime, &events, 1);
    let (claim_reached, release_claim) = runtime.gate_next_boundary_claim();
    let staged = runtime
        .admit_test_report(vec![0x20], vec![device("staged")])
        .unwrap();
    claim_reached.recv_timeout(Duration::from_secs(1)).unwrap();
    let queued = runtime
        .admit_test_report(vec![0x30], vec![device("queued")])
        .unwrap();
    focus_tx.send_replace(focused(1, "A"));
    focus_tx.send_replace(focused(2, "B"));
    release_claim.send(()).unwrap();

    for expected in [
        BackendEvent::Send("automatic".into(), vec![0x0b]),
        BackendEvent::Send("staged".into(), vec![0x20]),
        BackendEvent::Send("queued".into(), vec![0x30]),
    ] {
        assert_eq!(
            events.recv_timeout(Duration::from_secs(1)).unwrap(),
            expected
        );
    }
    assert_eq!(block_on(staged).unwrap().sent, 1);
    assert_eq!(block_on(queued).unwrap().sent, 1);
    owner.shutdown_and_join(Duration::from_secs(1));
    assert!(events.try_recv().is_err());
}

#[test]
fn shutdown_after_staging_cancels_tests_before_stopped_and_preempts_pending_focus() {
    let (focus_tx, focus_rx) = tokio::sync::watch::channel(ForegroundObservation::default());
    let (backend, events) = backend([ready(1)]);
    let (runtime, owner, progress) =
        start_with_progress(Some(config(0x10, &["automatic"])), focus_rx, backend);
    finish_startup(&runtime, &events, 1);
    let (claim_reached, release_claim) = runtime.gate_next_boundary_claim();
    let mut staged = Box::pin(runtime.test_report(vec![0x20], vec![device("staged")]));
    assert!(poll_once(staged.as_mut()).is_pending());
    claim_reached.recv_timeout(Duration::from_secs(1)).unwrap();
    let mut queued = Box::pin(runtime.test_report(vec![0x30], vec![device("queued")]));
    assert!(poll_once(queued.as_mut()).is_pending());
    runtime.request_hid_refresh().unwrap();
    focus_tx.send_replace(focused(1, "target"));
    runtime.request_shutdown();
    release_claim.send(()).unwrap();

    wait_for_phase(&runtime, RuntimePhase::Stopped);
    for response in [&mut staged, &mut queued] {
        assert_eq!(
            poll_once(response.as_mut()),
            Poll::Ready(Err(RuntimeRequestError::Cancelled))
        );
    }
    assert_eq!(focus_progress(&progress).latest_cancelled, Some(1));
    assert_eq!(
        block_on(runtime.test_report(vec![0x40], vec![device("late")])),
        Err(RuntimeRequestError::Unavailable)
    );
    owner.shutdown_and_join(Duration::from_secs(1));
    assert!(events.try_recv().is_err());
}

#[test]
fn startup_coalesces_pending_focus_and_dispatches_it_before_admitted_commands() {
    let (focus_tx, focus_rx) = tokio::sync::watch::channel(ForegroundObservation::default());
    let (mut backend, events) = backend([ready(1)]);
    let (refresh_started, refresh_release_tx) = gate_next_io(&mut backend.refresh_gates);
    let (runtime, owner) = start(
        Some(routed_config(&[("A", 0x0a), ("B", 0x0b)])),
        focus_rx,
        backend,
    );

    refresh_started.recv().unwrap();
    assert_eq!(events.recv().unwrap(), BackendEvent::RefreshStarted);
    focus_tx.send_replace(focused(1, "A"));
    focus_tx.send_replace(focused(2, "B"));
    let manual = runtime
        .admit_test_report(vec![0x20], vec![device("manual")])
        .unwrap();
    refresh_release_tx.send(()).unwrap();

    assert_eq!(events.recv().unwrap(), BackendEvent::RefreshFinished(1));
    assert_eq!(
        events.recv().unwrap(),
        BackendEvent::Send("automatic".to_string(), vec![0x0b])
    );
    assert_eq!(
        events.recv().unwrap(),
        BackendEvent::Send("manual".into(), vec![0x20])
    );
    assert_eq!(block_on(manual).unwrap().sent, 1);
    wait_for_phase(&runtime, RuntimePhase::Active);
    runtime.request_shutdown();
    owner.shutdown_and_join(Duration::from_secs(1));
    assert!(events.try_recv().is_err());
}

#[test]
fn replacement_before_a_focus_boundary_supplies_that_batch_snapshot() {
    let (focus_tx, focus_rx) = tokio::sync::watch::channel(ForegroundObservation::default());
    let (mut backend, events) = backend([ready(1)]);
    let (refresh_started, refresh_release_tx) = gate_next_io(&mut backend.refresh_gates);
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
    let (send_started, send_release_tx) = gate_next_io(&mut backend.send_gates);
    let (runtime, owner) = start(Some(config(0x10, &["automatic"])), focus_rx, backend);
    finish_startup(&runtime, &events, 1);

    let running = runtime
        .admit_test_report(vec![0x01], vec![device("running")])
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
        .admit_test_report(vec![0x02], vec![device("queued")])
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
    assert_eq!(block_on(running).unwrap().sent, 1);
    assert_eq!(block_on(queued).unwrap().sent, 1);
    owner.shutdown_and_join(Duration::from_secs(1));
}

#[test]
fn accepted_test_may_starve_during_focus_churn_then_recovers() {
    let (focus_tx, focus_rx) = tokio::sync::watch::channel(ForegroundObservation::default());
    let (mut backend, events) = backend([ready(1)]);
    let mut releases = Vec::new();
    let mut starts = Vec::new();
    for _ in 0..3 {
        let (started, release_tx) = gate_next_io(&mut backend.send_gates);
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
        .admit_test_report(vec![0x20], vec![device("manual")])
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
    assert_eq!(block_on(starved).unwrap().sent, 1);
    owner.shutdown_and_join(Duration::from_secs(1));
}

#[test]
fn no_action_focus_is_handled_without_retriggering_after_replacement() {
    let (focus_tx, focus_rx) = tokio::sync::watch::channel(ForegroundObservation::default());
    let (backend, events) = backend([ready(1), ready(2)]);
    let (runtime, owner, progress) =
        start_with_progress(Some(config(0x10, &["automatic"])), focus_rx, backend);
    finish_startup(&runtime, &events, 1);

    focus_tx.send_replace(focused(1, "unmatched"));
    let marker = runtime
        .admit_test_report(vec![0x30], vec![device("marker")])
        .unwrap();
    assert_eq!(
        events.recv().unwrap(),
        BackendEvent::Send("marker".to_string(), vec![0x30])
    );
    assert_eq!(block_on(marker).unwrap().sent, 1);
    assert_eq!(focus_progress(&progress).latest_handled, Some(1));

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
    let (started, release_tx) = gate_next_io(&mut backend.refresh_gates);
    let (runtime, owner) = start(Some(config(1, &["one"])), focus_rx, backend);

    finish_startup(&runtime, &events, 1);
    assert_eq!(runtime.status().phase, RuntimePhase::Degraded);
    let result = block_on(runtime.test_report(vec![0x44], vec![device("one")])).unwrap();
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
    let (send_started, send_release_tx) = gate_next_io(&mut backend.send_gates);
    backend.refresh_gates.push_back(None);
    let (refresh_started, refresh_release_tx) = gate_next_io(&mut backend.refresh_gates);
    let (runtime, owner) = start(Some(config(1, &["one"])), focus_rx, backend);
    finish_startup(&runtime, &events, 1);

    let test_runtime = runtime.clone();
    let test = thread::spawn(move || {
        block_on(test_runtime.test_report(vec![1], vec![device("one")])).unwrap()
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
fn manual_report_keeps_ordered_device_snapshots_and_continues_after_each_failure() {
    let (_focus_tx, focus_rx) = tokio::sync::watch::channel(ForegroundObservation::default());
    let (backend, events) = backend([ready(1)]);
    *backend.failed_devices.lock().unwrap() = vec!["third".into(), "first".into()];
    let sent_devices = backend.sent_devices.clone();
    let mut initial = config(0x10, &["first", "second", "third"]);
    // The first two durable devices alias a selector with different wire framing.
    initial.devices[0].report_id = 1;
    initial.devices[0].report_length = 2;
    initial.devices[1].report_id = 7;
    initial.devices[1].report_length = 6;
    let destinations = vec![
        initial.devices[2].clone(),
        initial.devices[0].clone(),
        initial.devices[1].clone(),
    ];
    let (runtime, owner) = start(Some(initial), focus_rx, backend);
    finish_startup(&runtime, &events, 1);
    let (claim_reached, release_claim) = runtime.gate_next_boundary_claim();
    let mut response = Box::pin(runtime.test_report(vec![0x45, 0x67], destinations.clone()));
    assert!(poll_once(response.as_mut()).is_pending());
    claim_reached.recv_timeout(Duration::from_secs(1)).unwrap();
    // Even removing the durable destinations cannot change an accepted Test snapshot.
    replace_config_for_test(&runtime, EditableConfig::default()).unwrap();
    release_claim.send(()).unwrap();

    let result = block_on(response).unwrap();
    assert_eq!(result.sent, 1);
    assert_eq!(result.failures.len(), 2);
    assert!(result.failures[0].starts_with("third: failed to write HID interface"));
    assert!(result.failures[1].starts_with("first: failed to write HID interface"));
    assert_eq!(*sent_devices.lock().unwrap(), destinations);
    for id in ["third", "first", "second"] {
        assert_eq!(
            events.recv_timeout(Duration::from_secs(1)).unwrap(),
            BackendEvent::Send(id.into(), vec![0x45, 0x67])
        );
    }
    owner.shutdown_and_join(Duration::from_secs(1));
    assert!(events.try_recv().is_err());
}

#[test]
fn admitted_test_refresh_and_test_execute_fifo_without_interleaving() {
    let (_focus_tx, focus_rx) = tokio::sync::watch::channel(ForegroundObservation::default());
    let (mut backend, events) = backend([ready(1), ready(2)]);
    let (send_started, send_release_tx) = gate_next_io(&mut backend.send_gates);
    let (runtime, owner) = start(Some(config(1, &["one"])), focus_rx, backend);
    finish_startup(&runtime, &events, 1);

    block_on(async {
        let first_runtime = runtime.clone();
        let first = tokio::spawn(async move {
            first_runtime
                .test_report(vec![1], vec![device("one"), device("two")])
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
                .test_report(vec![2], vec![device("three")])
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
            runtime.test_report(vec![3], vec![device("late")]).await,
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
    let (send_started, send_release_tx) = gate_next_io(&mut backend.send_gates);
    let (runtime, owner) = start(Some(config(1, &["running"])), focus_rx, backend);
    finish_startup(&runtime, &events, 1);

    let running = runtime
        .admit_test_report(vec![1], vec![device("running")])
        .unwrap();
    send_started.recv().unwrap();
    assert_eq!(
        events.recv().unwrap(),
        BackendEvent::Send("running".to_string(), vec![1])
    );

    let queued = (0..ORDINARY_COMMAND_CAPACITY)
        .map(|index| {
            runtime
                .admit_test_report(vec![2], vec![device(&format!("queued-{index}"))])
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        block_on(runtime.test_report(vec![3], vec![device("full")])),
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

    assert_eq!(block_on(running).unwrap().sent, 1);
    for response in queued {
        assert!(block_on(response).is_err());
    }
    owner.shutdown_and_join(Duration::from_secs(1));

    assert_eq!(runtime.status().phase, RuntimePhase::Stopped);
    assert!(events.try_recv().is_err());
}

#[test]
fn status_publication_does_not_regress_after_shutdown_starts() {
    let (_focus_tx, focus_rx) = tokio::sync::watch::channel(ForegroundObservation::default());
    let (mut backend, events) = backend([ready(1)]);
    let (send_started, send_release_tx) = gate_next_io(&mut backend.send_gates);
    let (runtime, owner) = start(Some(config(1, &["running"])), focus_rx, backend);
    finish_startup(&runtime, &events, 1);

    let running = runtime
        .admit_test_report(vec![1], vec![device("running")])
        .unwrap();
    send_started.recv().unwrap();
    assert!(matches!(events.recv().unwrap(), BackendEvent::Send(..)));
    runtime.request_shutdown();
    assert_eq!(runtime.status().phase, RuntimePhase::Stopping);

    send_release_tx.send(()).unwrap();
    assert_eq!(block_on(running).unwrap().sent, 1);
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
    let (send_started, send_release_tx) = gate_next_io(&mut backend.send_gates);
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
        block_on(test_runtime.test_report(vec![0x20], vec![device("manual")])).unwrap()
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
    let (send_started, send_release_tx) = gate_next_io(&mut backend.send_gates);
    let (runtime, owner, progress) =
        start_with_progress(Some(config(0x10, &["one", "two"])), focus_rx, backend);
    finish_startup(&runtime, &events, 1);

    focus_tx.send_replace(focused(1, "target first"));
    send_started.recv().unwrap();
    assert_eq!(
        events.recv().unwrap(),
        BackendEvent::Send("one".to_string(), vec![0x10])
    );
    assert_eq!(
        focus_progress(&progress),
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
        focus_progress(&progress),
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
        block_on(runtime.test_report(vec![1], vec![device("one"), device("two")])).unwrap();
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
    let empty = block_on(runtime.test_report(vec![1], Vec::new())).unwrap();
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
    block_on(runtime.test_report(vec![1], vec![device("one")])).unwrap();
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
    block_on(runtime.test_report(vec![1], vec![device("one")])).unwrap();
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

    block_on(runtime.test_report(vec![1], vec![device("one")])).unwrap();
    block_on(async { inventory.changed().await.unwrap() });
    assert_eq!(inventory.borrow().revision, 2);
    assert!(matches!(
        inventory.borrow().refresh_state,
        HidRefreshState::Failed { .. }
    ));
    assert_eq!(runtime.status().phase, RuntimePhase::Degraded);

    block_on(runtime.test_report(vec![1], vec![device("one")])).unwrap();
    block_on(async { inventory.changed().await.unwrap() });
    assert_eq!(inventory.borrow().revision, 3);
    assert_eq!(inventory.borrow().refresh_state, HidRefreshState::Ready);
    wait_for_phase(&runtime, RuntimePhase::Active);
    owner.shutdown_and_join(Duration::from_secs(1));
}

#[test]
fn missing_focus_source_or_configuration_is_unavailable() {
    for (has_config, source_available) in [(true, false), (false, true), (false, false)] {
        let (_focus_tx, focus_rx) = tokio::sync::watch::channel(ForegroundObservation::default());
        let (backend, events) = backend([ready(1)]);
        let source = if source_available {
            EventSourceState::Available
        } else {
            EventSourceState::Unavailable("hook failed".to_string())
        };
        let (runtime, owner) = AutomationRuntime::start_with_config(
            has_config.then(|| config(1, &["one"])),
            FocusInput::new(focus_rx, source),
            backend,
        )
        .unwrap();

        finish_startup(&runtime, &events, 1);
        assert_eq!(runtime.status().phase, RuntimePhase::Unavailable);
        let detail = runtime.status().detail.unwrap();
        assert_eq!(
            detail.contains("focused_window_changed: hook failed"),
            !source_available
        );
        assert_eq!(detail.contains("configuration is unavailable"), !has_config);
        runtime.request_shutdown();
        runtime.request_shutdown();
        owner.shutdown_and_join(Duration::from_secs(1));
        assert_eq!(runtime.status().phase, RuntimePhase::Stopped);
    }
}

#[test]
fn closed_source_finishes_its_last_event_and_keeps_commands_and_shutdown_responsive() {
    let (focus_tx, focus_rx) = tokio::sync::watch::channel(focused(1, "target"));
    drop(focus_tx);
    let (backend, events) = backend([ready(1), ready(2)]);
    let (runtime, owner, progress) =
        start_with_progress(Some(config(0x10, &["automatic"])), focus_rx, backend);

    finish_startup(&runtime, &events, 1);
    assert_eq!(
        events.recv_timeout(Duration::from_secs(1)).unwrap(),
        BackendEvent::Send("automatic".to_string(), vec![0x10])
    );
    wait_for_phase(&runtime, RuntimePhase::Unavailable);
    assert_eq!(focus_progress(&progress).latest_handled, Some(1));
    assert_eq!(
        runtime.status().detail.as_deref(),
        Some("focused_window_changed: event source closed")
    );

    let response = runtime
        .admit_test_report(vec![0x20], vec![device("manual")])
        .unwrap();
    assert_eq!(
        events.recv_timeout(Duration::from_secs(1)).unwrap(),
        BackendEvent::Send("manual".to_string(), vec![0x20])
    );
    assert_eq!(block_on(response).unwrap().sent, 1);
    assert_eq!(
        runtime.request_hid_refresh(),
        Ok(HidRefreshRequestResult::Queued)
    );
    assert_eq!(
        events.recv_timeout(Duration::from_secs(1)).unwrap(),
        BackendEvent::RefreshStarted
    );
    assert_eq!(
        events.recv_timeout(Duration::from_secs(1)).unwrap(),
        BackendEvent::RefreshFinished(2)
    );
    assert_eq!(runtime.status().phase, RuntimePhase::Unavailable);
    owner.shutdown_and_join(Duration::from_secs(1));
    assert_eq!(runtime.status().phase, RuntimePhase::Stopped);
    assert_eq!(focus_progress(&progress).latest_cancelled, None);
}

#[test]
fn worker_failure_cancels_accepted_reports_before_publishing_its_terminal_status() {
    for staged in [false, true] {
        for stopping in [false, true] {
            let (focus_tx, focus_rx) =
                tokio::sync::watch::channel(ForegroundObservation::default());
            let (mut backend, events) = backend([ready(1)]);
            backend.panic_on_send = true;
            let (send_started, send_release_tx) = gate_next_io(&mut backend.send_gates);
            let (runtime, owner) = start(Some(config(0x10, &["automatic"])), focus_rx, backend);
            finish_startup(&runtime, &events, 1);

            let gate = staged.then(|| runtime.gate_next_boundary_claim());
            let mut first = Box::pin(runtime.test_report(vec![0x20], vec![device("first")]));
            assert!(poll_once(first.as_mut()).is_pending());
            if let Some((claim_reached, release_claim)) = gate {
                claim_reached.recv_timeout(Duration::from_secs(1)).unwrap();
                // The failing automatic batch leaves this Test staged but unexecuted.
                focus_tx.send_replace(focused(1, "target"));
                release_claim.send(()).unwrap();
            }
            send_started.recv_timeout(Duration::from_secs(1)).unwrap();
            let mut queued = Box::pin(runtime.test_report(vec![0x30], vec![device("queued")]));
            assert!(poll_once(queued.as_mut()).is_pending());
            runtime.request_hid_refresh().unwrap();
            if stopping {
                runtime.request_shutdown();
            }
            send_release_tx.send(()).unwrap();

            wait_for_phase(
                &runtime,
                if stopping {
                    RuntimePhase::Stopped
                } else {
                    RuntimePhase::Unavailable
                },
            );
            for response in [&mut first, &mut queued] {
                assert_eq!(
                    poll_once(response.as_mut()),
                    Poll::Ready(Err(RuntimeRequestError::Cancelled))
                );
            }
            assert_eq!(
                runtime.request_hid_refresh(),
                Err(RuntimeRequestError::Unavailable)
            );
            assert_eq!(
                block_on(runtime.test_report(vec![0x40], vec![device("late")])),
                Err(RuntimeRequestError::Unavailable)
            );
            assert_eq!(
                events.recv_timeout(Duration::from_secs(1)).unwrap(),
                if staged {
                    BackendEvent::Send("automatic".into(), vec![0x10])
                } else {
                    BackendEvent::Send("first".into(), vec![0x20])
                }
            );
            owner.shutdown_and_join(Duration::from_secs(1));
            assert!(events.try_recv().is_err());
        }
    }
}

#[test]
fn worker_panic_closes_admission_and_clears_pending_refresh() {
    let (_focus_tx, focus_rx) = tokio::sync::watch::channel(ForegroundObservation::default());
    let (mut backend, _events) = backend([]);
    backend.panic_on_refresh = true;
    let (runtime, owner) = start(Some(config(1, &["one"])), focus_rx, backend);

    wait_for_phase(&runtime, RuntimePhase::Unavailable);
    assert!(runtime.request_hid_refresh().is_err());
    assert!(block_on(runtime.test_report(vec![1], vec![device("one")])).is_err());
    owner.shutdown_and_join(Duration::from_secs(1));
}
