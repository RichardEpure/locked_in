use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use super::*;
use crate::{
    components::workspace::automations::action_editor::action_destinations,
    config::{AutomationCase, ConfigStore, Device, StartWithWindows, StartWithWindowsOutcome},
};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct ConfirmedStartup;

impl StartWithWindows for ConfirmedStartup {
    fn reconcile(&self, desired: bool) -> StartWithWindowsOutcome {
        StartWithWindowsOutcome::confirmed(desired)
    }
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "locked-in-automation-publication-{}-{sequence}",
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

fn coordinator() -> (TestDirectory, Arc<ConfigStore>, Arc<ConfigCoordinator>) {
    let directory = TestDirectory::new();
    let store = Arc::new(ConfigStore::new(directory.0.join("config.toml")));
    let coordinator =
        ConfigCoordinator::initial_load(store.clone(), Arc::new(ConfirmedStartup)).unwrap();
    (directory, store, Arc::new(coordinator))
}

fn automation_with_case(id: &str) -> Automation {
    Automation {
        id: id.into(),
        name: "Durable".into(),
        cases: vec![AutomationCase {
            id: "case-1".into(),
            name: "Primary".into(),
            ..AutomationCase::default()
        }],
        ..Automation::default()
    }
}

fn seed_automation(
    coordinator: &ConfigCoordinator,
    automation: Automation,
) -> Arc<PublishedConfig> {
    let current = coordinator.current();
    coordinator
        .update(current.revision(), move |editable| {
            let mut next = editable.clone();
            next.automations.push(automation);
            next
        })
        .unwrap()
}

#[test]
fn new_and_duplicate_drafts_do_not_leak_and_cancel_restores_publication() {
    let (_directory, store, coordinator) = coordinator();
    let initial = coordinator.current();
    let mut new_draft = new_automation(&initial);
    new_draft.name = "Unsaved new".into();

    assert!(coordinator.current().editable().automations.is_empty());
    assert!(store.load_for_test().unwrap().automations.is_empty());
    assert_eq!(cancel_automation(&initial, &new_draft.id, true), None);

    let durable = seed_automation(&coordinator, automation_with_case("automation"));
    let mut duplicate = duplicate_automation(&durable, &durable.editable().automations[0]);
    duplicate.name = "Unsaved duplicate".into();
    let mut edited = durable.editable().automations[0].clone();
    edited.name = "Unsaved edit".into();

    assert_eq!(coordinator.current().editable().automations.len(), 1);
    assert_eq!(store.load_for_test().unwrap().automations.len(), 1);
    assert_eq!(
        cancel_automation(&coordinator.current(), &edited.id, false)
            .unwrap()
            .name,
        "Durable"
    );
    assert!(
        coordinator
            .current()
            .editable()
            .automations
            .iter()
            .all(|automation| automation.id != duplicate.id)
    );
}

#[test]
fn stale_save_preserves_the_draft_and_durable_automation() {
    let (_directory, _store, coordinator) = coordinator();
    let durable = seed_automation(&coordinator, automation_with_case("automation"));
    let mut draft = durable.editable().automations[0].clone();
    draft.name = "Preserved draft".into();
    let expected_draft = draft.clone();
    coordinator
        .update(durable.revision(), |editable| {
            let mut next = editable.clone();
            next.settings.start_minimized = !next.settings.start_minimized;
            next
        })
        .unwrap();

    let error = save_automation(&coordinator, durable.revision(), &draft, false).unwrap_err();

    assert!(matches!(
        error,
        AutomationCommitError::Coordinator(ConfigCoordinatorError::StaleRevision { .. })
    ));
    assert_eq!(draft, expected_draft);
    assert_eq!(
        coordinator.current().editable().automations[0].name,
        "Durable"
    );
}

#[test]
fn successful_save_and_delete_each_publish_exactly_one_revision() {
    let (_directory, store, coordinator) = coordinator();
    let initial = coordinator.current();
    let mut receiver = coordinator.subscribe();
    receiver.borrow_and_update();
    let draft = new_automation(&initial);

    let saved = save_automation(&coordinator, initial.revision(), &draft, true).unwrap();

    assert_eq!(saved.revision(), initial.revision() + 1);
    assert!(receiver.has_changed().unwrap());
    assert!(Arc::ptr_eq(&receiver.borrow_and_update(), &saved));
    assert!(!receiver.has_changed().unwrap());
    assert_eq!(store.load_for_test().unwrap(), *saved.editable().as_ref());

    let deleted = delete_automation(&coordinator, saved.revision(), &draft.id).unwrap();

    assert_eq!(deleted.revision(), saved.revision() + 1);
    assert!(receiver.has_changed().unwrap());
    assert!(Arc::ptr_eq(&receiver.borrow_and_update(), &deleted));
    assert!(!receiver.has_changed().unwrap());
    assert!(deleted.editable().automations.is_empty());
    assert_eq!(store.load_for_test().unwrap(), *deleted.editable().as_ref());
}

#[test]
fn captured_matcher_commits_durably_in_one_publication() {
    let (_directory, store, coordinator) = coordinator();
    let durable = seed_automation(&coordinator, automation_with_case("automation"));
    let mut receiver = coordinator.subscribe();
    receiver.borrow_and_update();
    let captured = FocusedWindow {
        title: Some("Editor".into()),
        class: Some("EditorWindow".into()),
        exe: Some(PathBuf::from(r"C:\Apps\editor.exe")),
    };

    let published = commit_captured_matcher(
        &coordinator,
        durable.revision(),
        "automation",
        "case-1",
        false,
        &captured,
    )
    .unwrap();

    assert_eq!(published.revision(), durable.revision() + 1);
    assert!(receiver.has_changed().unwrap());
    assert!(Arc::ptr_eq(&receiver.borrow_and_update(), &published));
    assert!(!receiver.has_changed().unwrap());
    let matcher = &published.editable().automations[0].cases[0].applications[0];
    assert_eq!(matcher.title.as_ref().unwrap().value, "Editor");
    assert_eq!(
        store.load_for_test().unwrap(),
        *published.editable().as_ref()
    );
}

#[test]
fn action_destinations_follow_the_supplied_publication_revision() {
    let (_directory, _store, coordinator) = coordinator();
    let initial = coordinator.current();
    assert!(action_destinations(&initial).is_empty());
    let published = coordinator
        .update(initial.revision(), |editable| {
            let mut next = editable.clone();
            next.devices.push(Device {
                id: "deck".into(),
                name: "Deck".into(),
                report_length: 8,
                ..Device::default()
            });
            next
        })
        .unwrap();

    assert_eq!(
        action_destinations(&published)
            .into_iter()
            .map(|device| device.id)
            .collect::<Vec<_>>(),
        ["deck"]
    );
}
