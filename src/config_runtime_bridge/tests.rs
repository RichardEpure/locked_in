use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use super::*;
use crate::{
    automation_runtime::{AutomationRuntime, FocusSourceState},
    config::{ConfigCoordinator, ConfigStore, StartWithWindows, StartWithWindowsOutcome},
    focused_window::ForegroundObservation,
    hid::{HidBackend, HidError, HidInventory, HidRefreshState},
};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct ConfirmedStartup;

impl StartWithWindows for ConfirmedStartup {
    fn reconcile(&self, desired: bool) -> StartWithWindowsOutcome {
        StartWithWindowsOutcome::confirmed(desired)
    }
}

struct ReadyBackend;

impl HidBackend for ReadyBackend {
    fn inventory(&self) -> HidInventory {
        HidInventory::default()
    }

    fn refresh(&mut self) -> HidInventory {
        HidInventory {
            revision: 1,
            refresh_state: HidRefreshState::Ready,
            rows: Vec::new(),
        }
    }

    fn send_report(
        &mut self,
        _device: &crate::config::Device,
        _report: &[u8],
    ) -> Result<(), HidError> {
        Ok(())
    }
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "locked-in-config-bridge-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn coordinator() -> (TestDirectory, Arc<ConfigCoordinator>) {
    let directory = TestDirectory::new();
    let store = Arc::new(ConfigStore::new(directory.0.join("config.toml")));
    let coordinator = ConfigCoordinator::initial_load(store, Arc::new(ConfirmedStartup)).unwrap();
    (directory, Arc::new(coordinator))
}

fn start_runtime(
    initial: Arc<crate::config::ActiveConfig>,
) -> (AutomationRuntime, crate::automation_runtime::RuntimeOwner) {
    let (_focus_tx, focus_rx) = tokio::sync::watch::channel(ForegroundObservation::default());
    AutomationRuntime::start_active(
        Some(initial),
        focus_rx,
        FocusSourceState::Available,
        ReadyBackend,
    )
    .unwrap()
}

fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(1);
    while !condition() {
        assert!(Instant::now() < deadline, "condition timed out");
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn bridge_applies_the_initial_receiver_state_and_latest_publication() {
    let (_directory, coordinator) = coordinator();
    let subscription = coordinator.subscribe();
    let initial = coordinator.current();
    let (runtime, runtime_owner) = start_runtime(initial.active().clone());
    let bridge = ConfigRuntimeBridge::start(runtime.clone(), subscription).unwrap();

    assert!(Arc::ptr_eq(
        runtime.active_config_snapshot().as_ref().unwrap(),
        initial.active()
    ));

    let next = coordinator
        .update(initial.revision(), |current| {
            let mut next = current.clone();
            next.settings.close_to_tray = !next.settings.close_to_tray;
            next
        })
        .unwrap();
    let latest = coordinator
        .update(next.revision(), |current| {
            let mut next = current.clone();
            next.settings.start_minimized = !next.settings.start_minimized;
            next
        })
        .unwrap();

    wait_until(|| {
        Arc::ptr_eq(
            runtime.active_config_snapshot().as_ref().unwrap(),
            latest.active(),
        )
    });
    bridge.shutdown_and_join();
    runtime_owner.shutdown_and_join(Duration::from_secs(1));
}

#[test]
fn synchronous_catch_up_closes_the_startup_subscription_gap_and_shutdown_stops_updates() {
    let (_directory, coordinator) = coordinator();
    let subscription = coordinator.subscribe();
    let initial = coordinator.current();
    let (runtime, runtime_owner) = start_runtime(initial.active().clone());
    let published_before_bridge = coordinator
        .update(initial.revision(), |current| {
            let mut next = current.clone();
            next.settings.close_to_tray = !next.settings.close_to_tray;
            next
        })
        .unwrap();

    let bridge = ConfigRuntimeBridge::start(runtime.clone(), subscription).unwrap();
    assert!(Arc::ptr_eq(
        runtime.active_config_snapshot().as_ref().unwrap(),
        published_before_bridge.active()
    ));
    bridge.shutdown_and_join();

    let after_shutdown = coordinator
        .update(published_before_bridge.revision(), |current| {
            let mut next = current.clone();
            next.settings.start_minimized = !next.settings.start_minimized;
            next
        })
        .unwrap();
    thread::sleep(Duration::from_millis(20));
    assert!(!Arc::ptr_eq(
        runtime.active_config_snapshot().as_ref().unwrap(),
        after_shutdown.active()
    ));
    assert!(Arc::ptr_eq(
        runtime.active_config_snapshot().as_ref().unwrap(),
        published_before_bridge.active()
    ));
    runtime_owner.shutdown_and_join(Duration::from_secs(1));
}
