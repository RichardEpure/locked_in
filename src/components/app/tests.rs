use std::{
    cell::RefCell,
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use super::*;
use crate::config::{
    ConfigCoordinator, ConfigStore, LogLevel, StartWithWindows, StartWithWindowsOutcome,
};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "locked-in-app-publication-{}-{sequence}",
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
struct WarningStartup(AtomicBool);

impl StartWithWindows for WarningStartup {
    fn reconcile(&self, desired: bool) -> StartWithWindowsOutcome {
        if self.0.swap(false, Ordering::SeqCst) {
            StartWithWindowsOutcome::warning(desired, "current publication warning")
        } else {
            StartWithWindowsOutcome::confirmed(desired)
        }
    }
}

#[test]
fn close_behavior_requires_both_the_published_preference_and_a_live_tray() {
    assert!(matches!(
        effective_close_behavior(false, false),
        WindowCloseBehaviour::WindowCloses
    ));
    assert!(matches!(
        effective_close_behavior(true, false),
        WindowCloseBehaviour::WindowCloses
    ));
    assert!(matches!(
        effective_close_behavior(false, true),
        WindowCloseBehaviour::WindowCloses
    ));
    assert!(matches!(
        effective_close_behavior(true, true),
        WindowCloseBehaviour::WindowHides
    ));
}

#[test]
fn later_settings_publications_cannot_enable_hiding_without_a_live_tray() {
    for close_to_tray in [true, false, true] {
        assert!(matches!(
            effective_close_behavior(close_to_tray, false),
            WindowCloseBehaviour::WindowCloses
        ));
    }
}

#[test]
fn startup_projection_applies_the_current_publication_after_a_subscription_race() {
    let directory = TestDirectory::new();
    let store = Arc::new(ConfigStore::new(directory.0.join("config.toml")));
    let startup = Arc::new(WarningStartup::default());
    let coordinator = ConfigCoordinator::initial_load(store, startup.clone()).unwrap();
    let initial = coordinator.current();
    let mut receiver = coordinator.subscribe();
    startup.0.store(true, Ordering::SeqCst);
    let current = coordinator
        .update(initial.revision(), |config| {
            let mut next = config.clone();
            next.settings.close_to_tray = false;
            next.settings.log_level = LogLevel::Debug;
            next
        })
        .unwrap();
    let projected = RefCell::new(None);

    project_current_publication(&mut receiver, true, |projection| {
        *projected.borrow_mut() = Some(projection);
    });

    let projected = projected.into_inner().unwrap();
    assert!(Arc::ptr_eq(&projected.publication, &current));
    assert_eq!(projected.log_level, LogLevel::Debug);
    assert!(matches!(
        projected.close_behavior,
        WindowCloseBehaviour::WindowCloses
    ));
    assert_eq!(projected.publication.warnings().len(), 1);
    assert!(!receiver.has_changed().unwrap());
}

#[test]
fn focused_window_bridge_projects_the_current_value_before_waiting_for_changes() {
    let current = win::WindowMetadata {
        title: Some("already focused".to_string()),
        ..win::WindowMetadata::default()
    };
    let (publisher, mut receiver) = tokio::sync::watch::channel(win::WindowMetadata::default());
    publisher.send_replace(current.clone());
    let projected = RefCell::new(None);

    publish_current_focused_window(&mut receiver, |focused| {
        *projected.borrow_mut() = Some(focused);
    });

    assert_eq!(projected.into_inner(), Some(current));
    assert!(!receiver.has_changed().unwrap());
}
