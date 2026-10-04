use std::{
    collections::VecDeque,
    fs,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use super::*;
use crate::{
    config::{Automation, ConfigCoordinator, ConfigCoordinatorError, ConfigStore, Device},
    platform::autostart::{LaunchAtLogin, LaunchAtLoginOutcome},
};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "locked-in-settings-view-{}-{sequence}",
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

#[derive(Default)]
struct StartupOutcomes(Mutex<VecDeque<LaunchAtLoginOutcome>>);

impl StartupOutcomes {
    fn push(&self, outcome: LaunchAtLoginOutcome) {
        self.0.lock().unwrap().push_back(outcome);
    }
}

impl LaunchAtLogin for StartupOutcomes {
    fn reconcile(&self, desired: bool) -> LaunchAtLoginOutcome {
        self.0
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| LaunchAtLoginOutcome::confirmed(desired))
    }
}

fn coordinator() -> (
    TestDirectory,
    Arc<ConfigStore>,
    Arc<StartupOutcomes>,
    Arc<ConfigCoordinator>,
) {
    let directory = TestDirectory::new();
    let store = Arc::new(ConfigStore::new(directory.0.join("config.toml")));
    let startup = Arc::new(StartupOutcomes::default());
    let coordinator =
        Arc::new(ConfigCoordinator::initial_load(store.clone(), startup.clone()).unwrap());
    (directory, store, startup, coordinator)
}

#[test]
fn save_uses_the_displayed_revision_and_publishes_durable_settings() {
    let (_directory, store, _startup, coordinator) = coordinator();
    let initial = coordinator.current();
    let mut settings = initial.editable().settings.clone();
    settings.close_to_tray = false;
    settings.log_level = LogLevel::Debug;

    let published = save_settings(&coordinator, initial.revision(), settings).unwrap();

    assert_eq!(published.revision(), initial.revision() + 1);
    assert!(!published.editable().settings.close_to_tray);
    assert_eq!(published.editable().settings.log_level, LogLevel::Debug);
    assert_eq!(
        store.load_for_test().unwrap(),
        *published.editable().as_ref()
    );
}

#[test]
fn reload_publishes_the_whole_external_configuration_through_the_coordinator() {
    let (_directory, store, _startup, coordinator) = coordinator();
    let initial = coordinator.current();
    let mut external = initial.editable().as_ref().clone();
    external.settings.start_minimized = false;
    external.devices.push(Device {
        id: "external-device".into(),
        name: "External device".into(),
        report_length: 32,
        ..Device::default()
    });
    external.automations.push(Automation {
        id: "external-automation".into(),
        name: "External automation".into(),
        ..Automation::default()
    });
    store.save_for_test(&external).unwrap();

    let published = coordinator.reload().unwrap();

    assert_eq!(published.revision(), initial.revision() + 1);
    assert!(!published.editable().settings.start_minimized);
    assert_eq!(*published.editable().as_ref(), external);
    assert_eq!(coordinator.current().revision(), published.revision());
}

#[test]
fn stale_save_and_reload_error_keep_the_durable_publication() {
    let (directory, _store, _startup, coordinator) = coordinator();
    let initial = coordinator.current();
    let current = coordinator
        .update(initial.revision(), {
            let mut next = initial.editable().as_ref().clone();
            next.settings.close_to_tray = false;
            next
        })
        .unwrap();

    let stale = save_settings(
        &coordinator,
        initial.revision(),
        initial.editable().settings.clone(),
    )
    .unwrap_err();
    assert!(matches!(
        stale,
        ConfigCoordinatorError::StaleRevision { .. }
    ));
    assert!(Arc::ptr_eq(&current, &coordinator.current()));

    fs::write(directory.0.join("config.toml"), "version = 1\n").unwrap();
    assert!(coordinator.reload().is_err());
    assert!(Arc::ptr_eq(&current, &coordinator.current()));
}

#[test]
fn failed_startup_change_persists_confirmed_state_and_surfaces_a_warning() {
    let (_directory, store, startup, coordinator) = coordinator();
    let initial = coordinator.current();
    startup.push(LaunchAtLoginOutcome::warning(false, "access denied"));
    let mut settings = initial.editable().settings.clone();
    settings.launch_at_login = true;
    settings.start_minimized = false;
    settings.close_to_tray = false;

    let published = save_settings(&coordinator, initial.revision(), settings).unwrap();
    let warning = config_warning_message(published.warnings()).unwrap();

    assert!(!published.editable().settings.launch_at_login);
    assert!(!published.editable().settings.start_minimized);
    assert!(!published.editable().settings.close_to_tray);
    assert_eq!(
        store.load_for_test().unwrap(),
        *published.editable().as_ref()
    );
    assert!(warning.contains("could not be enabled"));
    assert!(warning.contains("access denied"));
}

#[test]
fn reload_correction_warning_is_available_to_the_settings_ui() {
    let (_directory, store, startup, coordinator) = coordinator();
    let mut external = coordinator.current().editable().as_ref().clone();
    external.settings.launch_at_login = true;
    store.save_for_test(&external).unwrap();
    startup.push(LaunchAtLoginOutcome::warning(false, "registration missing"));

    let published = coordinator.reload().unwrap();
    let warning = config_warning_message(published.warnings()).unwrap();

    assert!(!published.editable().settings.launch_at_login);
    assert!(warning.contains("could not be enabled"));
    assert!(warning.contains("registration missing"));
}
