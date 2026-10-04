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
    config::{Automation, ConfigStore, SendAction},
    platform::autostart::{LaunchAtLogin, LaunchAtLoginOutcome},
};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct ConfirmedStartup;

impl LaunchAtLogin for ConfirmedStartup {
    fn reconcile(&self, desired: bool) -> LaunchAtLoginOutcome {
        LaunchAtLoginOutcome::confirmed(desired)
    }
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "locked-in-device-drafts-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn block(&self) {
        fs::remove_file(self.0.join("config.toml")).unwrap();
        fs::remove_dir(&self.0).unwrap();
        fs::write(&self.0, "configuration parent is blocked").unwrap();
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if self.0.is_file() {
            let _ = fs::remove_file(&self.0);
        } else {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

fn device(id: &str) -> Device {
    Device {
        id: id.into(),
        name: "Keyboard".into(),
        vid: 1,
        pid: 2,
        usage_page: 3,
        usage: 4,
        report_length: 8,
        report_id: 0,
    }
}

fn coordinator(initial: EditableConfig) -> (TestDirectory, Arc<ConfigCoordinator>) {
    let directory = TestDirectory::new();
    let store = Arc::new(ConfigStore::new(directory.0.join("config.toml")));
    store.save_for_test(&initial).unwrap();
    let coordinator = ConfigCoordinator::initial_load(store, Arc::new(ConfirmedStartup)).unwrap();
    (directory, Arc::new(coordinator))
}

fn referenced_config() -> EditableConfig {
    let mut config = EditableConfig::default();
    config.devices.push(device("keyboard"));
    config.automations.push(Automation {
        id: "typing".into(),
        name: "Typing".into(),
        otherwise_actions: vec![SendAction {
            id: "send".into(),
            device_ids: vec!["keyboard".into()],
            ..SendAction::default()
        }],
        ..Automation::default()
    });
    config
}

#[test]
fn new_draft_does_not_leak_into_durable_destinations_or_references_and_cancel_discards_it() {
    let (_directory, coordinator) = coordinator(EditableConfig::default());
    let published = coordinator.current();
    let mut draft = DeviceDraft::create(published.revision(), device("draft"));
    draft.edited.name = "Local edit".into();

    assert!(published.editable().devices.is_empty());
    assert!(device_references(published.editable(), "draft").is_empty());
    assert!(!draft.cancel(&published));
    assert!(coordinator.current().editable().devices.is_empty());
    assert_eq!(coordinator.current().revision(), published.revision());
}

#[test]
fn canceling_an_existing_edit_restores_the_current_durable_device() {
    let initial = referenced_config();
    let (_directory, coordinator) = coordinator(initial.clone());
    let published = coordinator.current();
    let mut draft = DeviceDraft::edit(published.revision(), initial.devices[0].clone());
    draft.edited.name = "Unsaved name".into();

    assert!(draft.cancel(&published));

    assert_eq!(draft.edited, initial.devices[0]);
    assert!(!draft.is_dirty());
    assert_eq!(coordinator.current().revision(), published.revision());
}

#[test]
fn clean_editor_refreshes_device_and_base_revision_from_a_new_publication() {
    let initial = referenced_config();
    let (_directory, coordinator) = coordinator(initial.clone());
    let published = coordinator.current();
    let mut draft = DeviceDraft::edit(published.revision(), initial.devices[0].clone());
    let refreshed = coordinator
        .update(published.revision(), {
            let mut next = published.editable().as_ref().clone();
            next.devices[0].name = "Published keyboard".into();
            next
        })
        .unwrap();

    assert!(draft.refresh_if_clean(&refreshed));
    assert_eq!(draft.edited.name, "Published keyboard");
    assert!(!draft.is_dirty());

    draft.edited.vid = 99;
    let saved = draft.save(&coordinator).unwrap();
    assert_eq!(saved.editable().devices[0].vid, 99);
}

#[test]
fn dirty_editor_retains_its_draft_and_base_revision_when_publication_changes() {
    let initial = referenced_config();
    let (_directory, coordinator) = coordinator(initial.clone());
    let published = coordinator.current();
    let mut draft = DeviceDraft::edit(published.revision(), initial.devices[0].clone());
    draft.edited.name = "Local keyboard".into();
    let before = draft.clone();
    let refreshed = coordinator
        .update(published.revision(), {
            let mut next = published.editable().as_ref().clone();
            next.devices[0].name = "Published keyboard".into();
            next
        })
        .unwrap();

    assert!(!draft.refresh_if_clean(&refreshed));
    assert_eq!(draft, before);

    let error = draft.save(&coordinator).unwrap_err();
    assert!(matches!(
        error,
        ConfigCoordinatorError::StaleRevision { .. }
    ));
    assert_eq!(draft, before);
}

#[test]
fn validation_and_store_failures_retain_the_complete_draft_without_publication() {
    let (directory, coordinator) = coordinator(EditableConfig::default());
    let published = coordinator.current();
    let subscription = coordinator.subscribe();
    let mut invalid = DeviceDraft::create(published.revision(), device("invalid"));
    invalid.edited.report_length = 0;
    let invalid_before = invalid.clone();

    let validation_error = invalid.save(&coordinator).unwrap_err();

    assert!(validation_error.to_string().contains("report length"));
    assert_eq!(invalid, invalid_before);
    assert!(!subscription.has_changed().unwrap());

    let mut unsaved = DeviceDraft::create(published.revision(), device("unsaved"));
    unsaved.edited.name = "Retained after disk failure".into();
    let unsaved_before = unsaved.clone();
    directory.block();

    let store_error = unsaved.save(&coordinator).unwrap_err();

    assert!(
        store_error
            .to_string()
            .contains("configuration save failed")
    );
    assert_eq!(unsaved, unsaved_before);
    assert_eq!(coordinator.current().revision(), published.revision());
    assert!(!subscription.has_changed().unwrap());
}

#[test]
fn stale_save_retains_the_draft_and_reports_the_current_revision() {
    let (_directory, coordinator) = coordinator(EditableConfig::default());
    let published = coordinator.current();
    let mut draft = DeviceDraft::create(published.revision(), device("stale"));
    draft.edited.name = "Still editable".into();
    let before = draft.clone();
    coordinator
        .update(published.revision(), {
            let mut next = published.editable().as_ref().clone();
            next.settings.start_minimized = false;
            next
        })
        .unwrap();

    let error = draft.save(&coordinator).unwrap_err();

    assert!(matches!(
        error,
        ConfigCoordinatorError::StaleRevision { .. }
    ));
    assert!(error.to_string().contains("current revision"));
    assert_eq!(draft, before);
    assert!(coordinator.current().editable().devices.is_empty());
}

#[test]
fn successful_save_publishes_the_device_exactly_once() {
    let (_directory, coordinator) = coordinator(EditableConfig::default());
    let published = coordinator.current();
    let mut subscription = coordinator.subscribe();
    let mut draft = DeviceDraft::create(published.revision(), device("keyboard"));

    let saved = draft.save(&coordinator).unwrap();

    assert_eq!(saved.editable().devices, [device("keyboard")]);
    assert!(!draft.is_new());
    assert!(!draft.is_dirty());
    assert!(subscription.has_changed().unwrap());
    assert!(Arc::ptr_eq(&saved, &subscription.borrow_and_update()));
    assert!(!subscription.has_changed().unwrap());
}

#[test]
fn references_block_delete_and_unreferenced_delete_publishes_once() {
    let initial = referenced_config();
    let (_directory, coordinator) = coordinator(initial.clone());
    let published = coordinator.current();
    let draft = DeviceDraft::edit(published.revision(), initial.devices[0].clone());
    let references = device_references(published.editable(), "keyboard");
    let blocked_subscription = coordinator.subscribe();

    let error = draft.delete(&coordinator, &references).unwrap_err();

    assert_eq!(error.to_string(), "Used by: Typing");
    assert!(!blocked_subscription.has_changed().unwrap());
    let without_reference = coordinator
        .update(published.revision(), {
            let mut next = published.editable().as_ref().clone();
            next.automations.clear();
            next
        })
        .unwrap();
    let draft = DeviceDraft::edit(
        without_reference.revision(),
        without_reference.editable().devices[0].clone(),
    );
    let mut delete_subscription = coordinator.subscribe();

    let deleted = draft.delete(&coordinator, &[]).unwrap().unwrap();

    assert!(deleted.editable().devices.is_empty());
    assert!(delete_subscription.has_changed().unwrap());
    assert!(Arc::ptr_eq(
        &deleted,
        &delete_subscription.borrow_and_update()
    ));
    assert!(!delete_subscription.has_changed().unwrap());
}
