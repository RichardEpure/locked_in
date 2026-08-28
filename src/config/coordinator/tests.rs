use std::{
    collections::VecDeque,
    sync::{
        Arc, Barrier, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

use super::*;
use crate::{
    config::{Automation, Device, Event, SendAction},
    focused_window::FocusedWindow,
};

type Events = Arc<Mutex<Vec<String>>>;
type ReentryHook = Arc<dyn Fn() + Send + Sync>;

struct SaveGate {
    entered: Arc<Barrier>,
    release: Arc<Barrier>,
}

struct FakeStore {
    disk: Mutex<EditableConfig>,
    events: Events,
    fail_next_load: AtomicBool,
    fail_next_save: AtomicBool,
    save_gate: Mutex<Option<SaveGate>>,
    save_hook: Mutex<Option<ReentryHook>>,
}

impl FakeStore {
    fn new(config: EditableConfig, events: Events) -> Self {
        Self {
            disk: Mutex::new(config),
            events,
            fail_next_load: AtomicBool::new(false),
            fail_next_save: AtomicBool::new(false),
            save_gate: Mutex::new(None),
            save_hook: Mutex::new(None),
        }
    }

    fn disk(&self) -> EditableConfig {
        self.disk.lock().unwrap().clone()
    }

    fn set_disk(&self, config: EditableConfig) {
        *self.disk.lock().unwrap() = config;
    }

    fn fail_load(&self) {
        self.fail_next_load.store(true, Ordering::SeqCst);
    }

    fn fail_save(&self) {
        self.fail_next_save.store(true, Ordering::SeqCst);
    }

    fn gate_save(&self, entered: Arc<Barrier>, release: Arc<Barrier>) {
        *self.save_gate.lock().unwrap() = Some(SaveGate { entered, release });
    }

    fn on_save(&self, hook: ReentryHook) {
        *self.save_hook.lock().unwrap() = Some(hook);
    }
}

impl CoordinatorStore for FakeStore {
    fn load(&self) -> Result<EditableConfig> {
        record(&self.events, "load");
        if self.fail_next_load.swap(false, Ordering::SeqCst) {
            anyhow::bail!("injected load failure");
        }
        Ok(self.disk())
    }

    fn save(&self, config: &EditableConfig) -> Result<()> {
        record(&self.events, "save");
        if let Some(hook) = self.save_hook.lock().unwrap().clone() {
            hook();
        }
        if let Some(gate) = self.save_gate.lock().unwrap().take() {
            gate.entered.wait();
            gate.release.wait();
        }
        if self.fail_next_save.swap(false, Ordering::SeqCst) {
            anyhow::bail!("injected save failure");
        }
        self.set_disk(config.clone());
        Ok(())
    }
}

struct FakeStartWithWindows {
    outcomes: Mutex<VecDeque<StartWithWindowsOutcome>>,
    events: Events,
    reconcile_hook: Mutex<Option<ReentryHook>>,
}

impl FakeStartWithWindows {
    fn new(events: Events) -> Self {
        Self {
            outcomes: Mutex::new(VecDeque::new()),
            events,
            reconcile_hook: Mutex::new(None),
        }
    }

    fn push(&self, outcome: StartWithWindowsOutcome) {
        self.outcomes.lock().unwrap().push_back(outcome);
    }

    fn on_reconcile(&self, hook: ReentryHook) {
        *self.reconcile_hook.lock().unwrap() = Some(hook);
    }
}

impl StartWithWindows for FakeStartWithWindows {
    fn reconcile(&self, desired: bool) -> StartWithWindowsOutcome {
        record(&self.events, &format!("reconcile:{desired}"));
        if let Some(hook) = self.reconcile_hook.lock().unwrap().clone() {
            hook();
        }
        self.outcomes
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| StartWithWindowsOutcome::confirmed(desired))
    }
}

fn record(events: &Events, event: &str) {
    events.lock().unwrap().push(event.to_string());
}

fn clear(events: &Events) {
    events.lock().unwrap().clear();
}

fn event_snapshot(events: &Events) -> Vec<String> {
    events.lock().unwrap().clone()
}

fn coordinator(
    config: EditableConfig,
) -> (
    Arc<ConfigCoordinator>,
    Arc<FakeStore>,
    Arc<FakeStartWithWindows>,
    Events,
) {
    let events = Events::default();
    let store = Arc::new(FakeStore::new(config, Arc::clone(&events)));
    let start_with_windows = Arc::new(FakeStartWithWindows::new(Arc::clone(&events)));
    let coordinator = Arc::new(
        ConfigCoordinator::initial_load_with_store(store.clone(), start_with_windows.clone())
            .unwrap(),
    );
    clear(&events);
    (coordinator, store, start_with_windows, events)
}

fn active_config() -> EditableConfig {
    let mut config = EditableConfig::default();
    config.devices.push(Device {
        id: "keyboard".into(),
        name: "Keyboard".into(),
        vid: 1,
        pid: 2,
        usage_page: 3,
        usage: 4,
        report_length: 8,
        report_id: 0,
    });
    config.automations.push(Automation {
        id: "automation".into(),
        name: "Automation".into(),
        enabled: true,
        event: Event::FocusedWindowChanged,
        cases: Vec::new(),
        otherwise_actions: vec![SendAction {
            id: "send".into(),
            label: "Send".into(),
            report: vec![0x42],
            device_ids: vec!["keyboard".into()],
        }],
    });
    config
}

fn assert_start_warning(warning: &ConfigWarning, expected_desired: bool, expected_confirmed: bool) {
    let ConfigWarning::StartWithWindows {
        desired,
        confirmed,
        message,
    } = warning
    else {
        panic!("expected Start with Windows warning");
    };
    assert_eq!(*desired, expected_desired);
    assert_eq!(*confirmed, Some(expected_confirmed));
    assert!(message.is_some());
}

fn assert_rollback_warning(
    warning: &ConfigWarning,
    expected_target: bool,
    expected_attempted: bool,
    expected_confirmed: Option<bool>,
) {
    let ConfigWarning::StartWithWindowsRollback {
        target,
        attempted,
        confirmed,
        ..
    } = warning
    else {
        panic!("expected Start with Windows rollback warning");
    };
    assert_eq!(*target, expected_target);
    assert_eq!(*attempted, expected_attempted);
    assert_eq!(*confirmed, expected_confirmed);
}

#[test]
fn initial_load_publishes_one_matching_immutable_revision() {
    let config = active_config();
    let events = Events::default();
    let store = Arc::new(FakeStore::new(config.clone(), Arc::clone(&events)));
    let start_with_windows = Arc::new(FakeStartWithWindows::new(Arc::clone(&events)));

    let coordinator = ConfigCoordinator::initial_load_with_store(store, start_with_windows)
        .expect("initial load");
    let current = coordinator.current();
    let subscription = coordinator.subscribe();
    let subscribed = subscription.borrow().clone();

    assert_eq!(current.revision(), INITIAL_REVISION);
    assert_eq!(current.editable().as_ref(), &config);
    assert!(Arc::ptr_eq(&current, &subscribed));
    assert!(Arc::ptr_eq(current.editable(), subscribed.editable()));
    assert!(Arc::ptr_eq(current.active(), subscribed.active()));
    assert!(current.warnings().is_empty());
    let dispatches = current.active().evaluate_window(&FocusedWindow::default());
    assert_eq!(dispatches.len(), 1);
    assert_eq!(dispatches[0].report(), &[0x42]);
    assert_eq!(event_snapshot(&events), ["load", "reconcile:false"]);
}

#[test]
fn update_orders_candidate_reconciliation_save_then_one_publication() {
    let (coordinator, store, _, events) = coordinator(EditableConfig::default());
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    store.gate_save(Arc::clone(&entered), Arc::clone(&release));
    let mut subscription = coordinator.subscribe();

    let update = thread::spawn({
        let coordinator = Arc::clone(&coordinator);
        let events = Arc::clone(&events);
        move || {
            coordinator.update(INITIAL_REVISION, |current| {
                record(&events, "build");
                let mut candidate = current.clone();
                candidate.settings.start_with_windows = true;
                candidate
            })
        }
    });

    entered.wait();
    assert_eq!(coordinator.current().revision(), INITIAL_REVISION);
    assert!(!subscription.has_changed().unwrap());
    assert_eq!(event_snapshot(&events), ["build", "reconcile:true", "save"]);

    release.wait();
    let published = update.join().unwrap().unwrap();
    assert_eq!(published.revision(), INITIAL_REVISION + 1);
    assert!(subscription.has_changed().unwrap());
    let observed = subscription.borrow_and_update().clone();
    assert!(Arc::ptr_eq(&published, &observed));
    assert!(!subscription.has_changed().unwrap());
    assert!(store.disk().settings.start_with_windows);
}

#[test]
fn stale_revision_is_rejected_before_building_or_touching_dependencies() {
    let (coordinator, _, _, events) = coordinator(EditableConfig::default());
    coordinator
        .update(INITIAL_REVISION, |current| {
            let mut candidate = current.clone();
            candidate.settings.start_minimized = false;
            candidate
        })
        .unwrap();
    clear(&events);
    let built = AtomicBool::new(false);

    let error = coordinator
        .update(INITIAL_REVISION, |current| {
            built.store(true, Ordering::SeqCst);
            current.clone()
        })
        .unwrap_err();

    assert!(matches!(
        error,
        ConfigCoordinatorError::StaleRevision {
            expected: INITIAL_REVISION,
            actual,
        } if actual == INITIAL_REVISION + 1
    ));
    assert!(!built.load(Ordering::SeqCst));
    assert!(event_snapshot(&events).is_empty());
}

#[test]
fn builder_can_reenter_the_coordinator_without_deadlock() {
    let (coordinator, store, _, events) = coordinator(EditableConfig::default());

    let error = coordinator
        .update(INITIAL_REVISION, |outer_base| {
            let nested = coordinator
                .update(INITIAL_REVISION, |current| {
                    let mut candidate = current.clone();
                    candidate.settings.start_minimized = false;
                    candidate
                })
                .unwrap();
            assert_eq!(nested.revision(), INITIAL_REVISION + 1);

            let mut candidate = outer_base.clone();
            candidate.settings.close_to_tray = false;
            candidate
        })
        .unwrap_err();

    assert!(matches!(
        error,
        ConfigCoordinatorError::StaleRevision {
            expected: INITIAL_REVISION,
            actual,
        } if actual == INITIAL_REVISION + 1
    ));
    assert!(!store.disk().settings.start_minimized);
    assert!(store.disk().settings.close_to_tray);
    assert_eq!(event_snapshot(&events), ["reconcile:false", "save"]);
}

#[test]
fn adapter_and_store_mutation_reentry_is_rejected_without_deadlock() {
    let (coordinator, store, start_with_windows, _) = coordinator(EditableConfig::default());
    let adapter_rejected = Arc::new(AtomicBool::new(false));
    let store_rejected = Arc::new(AtomicBool::new(false));
    let nested_builder_called = Arc::new(AtomicBool::new(false));

    start_with_windows.on_reconcile({
        let coordinator = Arc::downgrade(&coordinator);
        let adapter_rejected = Arc::clone(&adapter_rejected);
        Arc::new(move || {
            let coordinator = coordinator.upgrade().unwrap();
            assert_eq!(coordinator.current().revision(), INITIAL_REVISION);
            adapter_rejected.store(
                matches!(
                    coordinator.reload(),
                    Err(ConfigCoordinatorError::ReentrantOperation)
                ),
                Ordering::SeqCst,
            );
        })
    });
    store.on_save({
        let coordinator = Arc::downgrade(&coordinator);
        let store_rejected = Arc::clone(&store_rejected);
        let nested_builder_called = Arc::clone(&nested_builder_called);
        Arc::new(move || {
            let coordinator = coordinator.upgrade().unwrap();
            store_rejected.store(
                matches!(
                    coordinator.update(INITIAL_REVISION, |current| {
                        nested_builder_called.store(true, Ordering::SeqCst);
                        current.clone()
                    }),
                    Err(ConfigCoordinatorError::ReentrantOperation)
                ),
                Ordering::SeqCst,
            );
        })
    });

    let published = coordinator
        .update(INITIAL_REVISION, |current| {
            let mut candidate = current.clone();
            candidate.settings.close_to_tray = false;
            candidate
        })
        .unwrap();

    assert_eq!(published.revision(), INITIAL_REVISION + 1);
    assert!(adapter_rejected.load(Ordering::SeqCst));
    assert!(store_rejected.load(Ordering::SeqCst));
    assert!(!nested_builder_called.load(Ordering::SeqCst));
}

#[test]
fn validation_or_compilation_failure_never_calls_the_os_or_store() {
    let (coordinator, store, _, events) = coordinator(EditableConfig::default());
    let before = coordinator.current();
    let subscription = coordinator.subscribe();

    let error = coordinator
        .update(INITIAL_REVISION, |current| {
            record(&events, "build");
            let mut candidate = current.clone();
            candidate.settings.start_with_windows = true;
            candidate.devices.push(Device::default());
            candidate
        })
        .unwrap_err();

    assert!(matches!(
        error,
        ConfigCoordinatorError::InvalidConfig { .. }
    ));
    assert!(error.warnings().is_empty());
    assert!(Arc::ptr_eq(&before, &coordinator.current()));
    assert_eq!(store.disk(), EditableConfig::default());
    assert!(!subscription.has_changed().unwrap());
    assert_eq!(event_snapshot(&events), ["build"]);
}

#[test]
fn store_failure_keeps_prior_publication_revision_and_disk() {
    let (coordinator, store, _, events) = coordinator(EditableConfig::default());
    store.fail_save();
    let before = coordinator.current();
    let subscription = coordinator.subscribe();

    let error = coordinator
        .update(INITIAL_REVISION, |current| {
            record(&events, "build");
            let mut candidate = current.clone();
            candidate.settings.close_to_tray = false;
            candidate
        })
        .unwrap_err();

    assert!(matches!(
        error,
        ConfigCoordinatorError::Store {
            operation: StoreOperation::Save,
            ..
        }
    ));
    assert!(Arc::ptr_eq(&before, &coordinator.current()));
    assert_eq!(store.disk(), EditableConfig::default());
    assert!(!subscription.has_changed().unwrap());
    assert_eq!(
        event_snapshot(&events),
        ["build", "reconcile:false", "save"]
    );
    assert_rollback_warning(&error.warnings()[0], false, false, Some(false));
}

#[test]
fn successful_os_change_followed_by_save_failure_rolls_back() {
    let (coordinator, store, start_with_windows, events) = coordinator(EditableConfig::default());
    start_with_windows.push(StartWithWindowsOutcome::confirmed(true));
    start_with_windows.push(StartWithWindowsOutcome::confirmed(false));
    store.fail_save();
    let before = coordinator.current();
    let subscription = coordinator.subscribe();

    let error = coordinator
        .update(INITIAL_REVISION, |current| {
            let mut candidate = current.clone();
            candidate.settings.start_with_windows = true;
            candidate.settings.close_to_tray = false;
            candidate
        })
        .unwrap_err();

    assert!(matches!(
        error,
        ConfigCoordinatorError::Store {
            operation: StoreOperation::Save,
            ..
        }
    ));
    assert_rollback_warning(&error.warnings()[0], false, true, Some(false));
    assert!(Arc::ptr_eq(&before, &coordinator.current()));
    assert_eq!(store.disk(), EditableConfig::default());
    assert!(!subscription.has_changed().unwrap());
    assert_eq!(
        event_snapshot(&events),
        ["reconcile:true", "save", "reconcile:false"]
    );
}

#[test]
fn unconfirmed_rollback_is_reported_without_claiming_restoration() {
    let (coordinator, store, start_with_windows, events) = coordinator(EditableConfig::default());
    start_with_windows.push(StartWithWindowsOutcome::confirmed(true));
    start_with_windows.push(StartWithWindowsOutcome::unconfirmed(
        "rollback could not be queried",
    ));
    store.fail_save();

    let error = coordinator
        .update(INITIAL_REVISION, |current| {
            let mut candidate = current.clone();
            candidate.settings.start_with_windows = true;
            candidate
        })
        .unwrap_err();

    assert!(matches!(
        error,
        ConfigCoordinatorError::Store {
            operation: StoreOperation::Save,
            ..
        }
    ));
    assert_rollback_warning(&error.warnings()[0], false, true, None);
    assert!(matches!(
        &error.warnings()[0],
        ConfigWarning::StartWithWindowsRollback {
            message: Some(message),
            ..
        } if message == "rollback could not be queried"
    ));
    assert_eq!(coordinator.current().revision(), INITIAL_REVISION);
    assert_eq!(store.disk(), EditableConfig::default());
    assert_eq!(
        event_snapshot(&events),
        ["reconcile:true", "save", "reconcile:false"]
    );
}

#[test]
fn failed_rollback_reports_the_confirmed_actual_state() {
    let (coordinator, store, start_with_windows, events) = coordinator(EditableConfig::default());
    start_with_windows.push(StartWithWindowsOutcome::confirmed(true));
    start_with_windows.push(StartWithWindowsOutcome::warning(true, "still enabled"));
    store.fail_save();

    let error = coordinator
        .update(INITIAL_REVISION, |current| {
            let mut candidate = current.clone();
            candidate.settings.start_with_windows = true;
            candidate
        })
        .unwrap_err();

    assert_rollback_warning(&error.warnings()[0], false, true, Some(true));
    assert!(matches!(
        &error.warnings()[0],
        ConfigWarning::StartWithWindowsRollback {
            message: Some(message),
            ..
        } if message == "still enabled"
    ));
    assert_eq!(coordinator.current().revision(), INITIAL_REVISION);
    assert_eq!(store.disk(), EditableConfig::default());
    assert_eq!(
        event_snapshot(&events),
        ["reconcile:true", "save", "reconcile:false"]
    );
}

#[test]
fn unconfirmed_requested_state_is_never_saved_or_published() {
    let (coordinator, store, start_with_windows, events) = coordinator(EditableConfig::default());
    start_with_windows.push(StartWithWindowsOutcome::unconfirmed(
        "registration state unavailable",
    ));
    start_with_windows.push(StartWithWindowsOutcome::confirmed(false));
    let before = coordinator.current();
    let subscription = coordinator.subscribe();

    let error = coordinator
        .update(INITIAL_REVISION, |current| {
            let mut candidate = current.clone();
            candidate.settings.start_with_windows = true;
            candidate.settings.close_to_tray = false;
            candidate
        })
        .unwrap_err();

    assert!(matches!(
        error,
        ConfigCoordinatorError::UnconfirmedStartWithWindows { .. }
    ));
    assert!(matches!(
        &error.warnings()[0],
        ConfigWarning::StartWithWindows {
            desired: true,
            confirmed: None,
            ..
        }
    ));
    assert_rollback_warning(&error.warnings()[1], false, true, Some(false));
    assert!(Arc::ptr_eq(&before, &coordinator.current()));
    assert_eq!(store.disk(), EditableConfig::default());
    assert!(!subscription.has_changed().unwrap());
    assert_eq!(
        event_snapshot(&events),
        ["reconcile:true", "reconcile:false"]
    );
}

#[test]
fn concurrent_updates_are_serialized_and_the_loser_is_stale() {
    let (coordinator, store, _, _) = coordinator(EditableConfig::default());
    let mut subscription = coordinator.subscribe();
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    store.gate_save(Arc::clone(&entered), Arc::clone(&release));

    let first = thread::spawn({
        let coordinator = Arc::clone(&coordinator);
        move || {
            coordinator.update(INITIAL_REVISION, |current| {
                let mut candidate = current.clone();
                candidate.settings.start_minimized = false;
                candidate
            })
        }
    });
    entered.wait();

    let (second_built, second_built_rx) = mpsc::channel();
    let second = thread::spawn({
        let coordinator = Arc::clone(&coordinator);
        move || {
            coordinator.update(INITIAL_REVISION, |current| {
                second_built.send(()).unwrap();
                let mut candidate = current.clone();
                candidate.settings.close_to_tray = false;
                candidate
            })
        }
    });

    second_built_rx.recv().unwrap();
    release.wait();
    let first_publication = first.join().unwrap().unwrap();
    let second_error = second.join().unwrap().unwrap_err();

    assert_eq!(first_publication.revision(), INITIAL_REVISION + 1);
    assert!(matches!(
        second_error,
        ConfigCoordinatorError::StaleRevision { .. }
    ));
    assert!(!store.disk().settings.start_minimized);
    assert!(store.disk().settings.close_to_tray);
    assert!(subscription.has_changed().unwrap());
    subscription.borrow_and_update();
    assert!(!subscription.has_changed().unwrap());
}

#[test]
fn reload_failure_leaves_prior_publication_unchanged() {
    let (coordinator, store, _, events) = coordinator(EditableConfig::default());
    let before = coordinator.current();
    let subscription = coordinator.subscribe();
    store.fail_load();

    let error = coordinator.reload().unwrap_err();

    assert!(matches!(
        error,
        ConfigCoordinatorError::Store {
            operation: StoreOperation::Reload,
            ..
        }
    ));
    assert!(Arc::ptr_eq(&before, &coordinator.current()));
    assert!(!subscription.has_changed().unwrap());
    assert_eq!(event_snapshot(&events), ["load"]);
}

#[test]
fn reload_with_unconfirmed_startup_state_rolls_back_without_saving_or_publishing() {
    let (coordinator, store, start_with_windows, events) = coordinator(EditableConfig::default());
    let mut external = EditableConfig::default();
    external.settings.start_with_windows = true;
    external.settings.close_to_tray = false;
    store.set_disk(external.clone());
    start_with_windows.push(StartWithWindowsOutcome::unconfirmed(
        "startup state unavailable",
    ));
    start_with_windows.push(StartWithWindowsOutcome::confirmed(false));
    let before = coordinator.current();
    let subscription = coordinator.subscribe();

    let error = coordinator.reload().unwrap_err();

    assert!(matches!(
        error,
        ConfigCoordinatorError::UnconfirmedStartWithWindows { .. }
    ));
    assert!(matches!(
        &error.warnings()[0],
        ConfigWarning::StartWithWindows {
            desired: true,
            confirmed: None,
            ..
        }
    ));
    assert_rollback_warning(&error.warnings()[1], false, true, Some(false));
    assert_eq!(store.disk(), external);
    assert!(Arc::ptr_eq(&before, &coordinator.current()));
    assert!(!subscription.has_changed().unwrap());
    assert_eq!(
        event_snapshot(&events),
        ["load", "reconcile:true", "reconcile:false"]
    );
}

#[test]
fn reload_reports_when_rollback_after_unconfirmed_state_is_also_unconfirmed() {
    let (coordinator, store, start_with_windows, events) = coordinator(EditableConfig::default());
    let mut external = EditableConfig::default();
    external.settings.start_with_windows = true;
    store.set_disk(external.clone());
    start_with_windows.push(StartWithWindowsOutcome::unconfirmed(
        "startup state unavailable",
    ));
    start_with_windows.push(StartWithWindowsOutcome::unconfirmed(
        "rollback state unavailable",
    ));

    let error = coordinator.reload().unwrap_err();

    assert!(matches!(
        error,
        ConfigCoordinatorError::UnconfirmedStartWithWindows { .. }
    ));
    assert_rollback_warning(&error.warnings()[1], false, true, None);
    assert!(matches!(
        &error.warnings()[1],
        ConfigWarning::StartWithWindowsRollback {
            message: Some(message),
            ..
        } if message == "rollback state unavailable"
    ));
    assert_eq!(store.disk(), external);
    assert_eq!(coordinator.current().revision(), INITIAL_REVISION);
    assert_eq!(
        event_snapshot(&events),
        ["load", "reconcile:true", "reconcile:false"]
    );
}

#[test]
fn initial_load_reports_unconfirmed_state_without_fabricating_a_rollback_target() {
    let events = Events::default();
    let mut initial = EditableConfig::default();
    initial.settings.start_with_windows = true;
    let store = Arc::new(FakeStore::new(initial.clone(), Arc::clone(&events)));
    let start_with_windows = Arc::new(FakeStartWithWindows::new(Arc::clone(&events)));
    start_with_windows.push(StartWithWindowsOutcome::unconfirmed(
        "startup state unavailable",
    ));

    let error = match ConfigCoordinator::initial_load_with_store(store.clone(), start_with_windows)
    {
        Ok(_) => panic!("unconfirmed initial state must fail"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        ConfigCoordinatorError::UnconfirmedStartWithWindows { .. }
    ));
    assert_eq!(error.warnings().len(), 1);
    assert!(matches!(
        &error.warnings()[0],
        ConfigWarning::StartWithWindows {
            desired: true,
            confirmed: None,
            ..
        }
    ));
    assert_eq!(store.disk(), initial);
    assert_eq!(event_snapshot(&events), ["load", "reconcile:true"]);
}

#[test]
fn failed_enable_preserves_other_edits_and_persists_confirmed_false() {
    let (coordinator, store, start_with_windows, _) = coordinator(EditableConfig::default());
    start_with_windows.push(StartWithWindowsOutcome::warning(false, "access denied"));

    let published = coordinator
        .update(INITIAL_REVISION, |current| {
            let mut candidate = current.clone();
            candidate.settings.start_with_windows = true;
            candidate.settings.close_to_tray = false;
            candidate
        })
        .unwrap();

    assert!(!published.editable().settings.start_with_windows);
    assert!(!published.editable().settings.close_to_tray);
    assert_eq!(store.disk(), *published.editable().as_ref());
    assert_start_warning(&published.warnings()[0], true, false);
}

#[test]
fn failed_disable_preserves_other_edits_and_persists_confirmed_true() {
    let mut initial = EditableConfig::default();
    initial.settings.start_with_windows = true;
    let (coordinator, store, start_with_windows, _) = coordinator(initial);
    start_with_windows.push(StartWithWindowsOutcome::warning(true, "removal failed"));

    let published = coordinator
        .update(INITIAL_REVISION, |current| {
            let mut candidate = current.clone();
            candidate.settings.start_with_windows = false;
            candidate.settings.start_minimized = false;
            candidate
        })
        .unwrap();

    assert!(published.editable().settings.start_with_windows);
    assert!(!published.editable().settings.start_minimized);
    assert_eq!(store.disk(), *published.editable().as_ref());
    assert_start_warning(&published.warnings()[0], false, true);
}

#[test]
fn initial_load_and_reload_save_applied_setting_corrections_before_publication() {
    let events = Events::default();
    let mut initial = EditableConfig::default();
    initial.settings.start_with_windows = true;
    let store = Arc::new(FakeStore::new(initial, Arc::clone(&events)));
    let start_with_windows = Arc::new(FakeStartWithWindows::new(Arc::clone(&events)));
    start_with_windows.push(StartWithWindowsOutcome::warning(false, "startup failed"));

    let coordinator =
        ConfigCoordinator::initial_load_with_store(store.clone(), start_with_windows.clone())
            .expect("corrected initial load");
    assert!(!coordinator.current().editable().settings.start_with_windows);
    assert!(!store.disk().settings.start_with_windows);
    assert_start_warning(&coordinator.current().warnings()[0], true, false);
    assert_eq!(event_snapshot(&events), ["load", "reconcile:true", "save"]);

    clear(&events);
    let mut external = EditableConfig::default();
    external.settings.start_with_windows = true;
    external.settings.close_to_tray = false;
    store.set_disk(external);
    start_with_windows.push(StartWithWindowsOutcome::warning(false, "reload failed"));

    let reloaded = coordinator.reload().unwrap();
    assert_eq!(reloaded.revision(), INITIAL_REVISION + 1);
    assert!(!reloaded.editable().settings.start_with_windows);
    assert!(!reloaded.editable().settings.close_to_tray);
    assert_eq!(store.disk(), *reloaded.editable().as_ref());
    assert_start_warning(&reloaded.warnings()[0], true, false);
    assert_eq!(event_snapshot(&events), ["load", "reconcile:true", "save"]);
}

#[test]
fn correction_save_failure_reports_warning_without_claiming_disk_or_publication_changed() {
    let (coordinator, store, start_with_windows, events) = coordinator(EditableConfig::default());
    let mut external = EditableConfig::default();
    external.settings.start_with_windows = true;
    external.settings.close_to_tray = false;
    store.set_disk(external.clone());
    store.fail_save();
    start_with_windows.push(StartWithWindowsOutcome::warning(
        false,
        "registration missing",
    ));
    let before = coordinator.current();
    let subscription = coordinator.subscribe();

    let error = coordinator.reload().unwrap_err();

    assert!(matches!(
        error,
        ConfigCoordinatorError::Store {
            operation: StoreOperation::CorrectionSave,
            ..
        }
    ));
    assert_start_warning(&error.warnings()[0], true, false);
    assert!(
        error
            .to_string()
            .contains("disk was not reported as corrected")
    );
    assert_eq!(store.disk(), external);
    assert!(Arc::ptr_eq(&before, &coordinator.current()));
    assert_eq!(coordinator.current().revision(), INITIAL_REVISION);
    assert!(!subscription.has_changed().unwrap());
    assert_eq!(event_snapshot(&events), ["load", "reconcile:true", "save"]);
}
