use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use super::super::editing::{duplicate_automation, new_automation};
use super::*;
use crate::{
    config::ConfigStore,
    platform::autostart::{LaunchAtLogin, LaunchAtLoginOutcome},
};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    path: PathBuf,
    coordinator: ConfigCoordinator,
}

struct ConfirmedStartup;
impl LaunchAtLogin for ConfirmedStartup {
    fn reconcile(&self, desired: bool) -> LaunchAtLoginOutcome {
        LaunchAtLoginOutcome::confirmed(desired)
    }
}

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "locked-in-automation-draft-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        let coordinator = ConfigCoordinator::initial_load(
            Arc::new(ConfigStore::new(path.join("config.toml"))),
            Arc::new(ConfirmedStartup),
        )
        .unwrap();
        Self { path, coordinator }
    }

    fn draft(&self) -> AutomationDraft {
        let current = self.coordinator.current();
        AutomationDraft::create(current.revision(), new_automation(&current))
    }

    fn rename_durable(&self, name: &str) -> Arc<PublishedConfig> {
        let current = self.coordinator.current();
        let mut candidate = current.editable().as_ref().clone();
        candidate.automations[0].name = name.into();
        self.coordinator
            .update(current.revision(), candidate)
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn creation_and_duplicate_are_dirty_without_edits_and_cancel_never_publishes() {
    let fixture = Fixture::new();
    let mut draft = fixture.draft();
    let initial = fixture.coordinator.current();
    assert!(draft.is_new() && draft.is_dirty());
    assert!(!draft.cancel(&initial));
    assert!(draft.delete(&fixture.coordinator).unwrap().is_none());
    assert_eq!(fixture.coordinator.current().revision(), initial.revision());
    let saved = draft.save(&fixture.coordinator).unwrap();
    assert!(!draft.is_new() && !draft.is_dirty());
    let mut copy = AutomationDraft::create(
        saved.revision(),
        duplicate_automation(&saved, &draft.edited),
    );
    assert!(copy.is_new() && copy.is_dirty());
    assert!(!copy.cancel(&saved));
    assert_eq!(
        fixture.coordinator.current().editable().automations.len(),
        1
    );
}

#[test]
fn first_edit_and_baseline_survive_publication_but_clean_editors_refresh() {
    let fixture = Fixture::new();
    let mut draft = fixture.draft();
    draft.save(&fixture.coordinator).unwrap();
    let mut clean = draft.clone();
    draft.edited.name = "First edit".into();
    let expected = draft.clone();
    let published = fixture.rename_durable("Other writer");
    assert!(!draft.refresh_if_clean(&published));
    assert_eq!(draft, expected);
    assert!(clean.refresh_if_clean(&published));
    assert_eq!(clean.edited.name, "Other writer");
    assert!(!clean.is_dirty());
}

#[test]
fn stale_save_only_advances_retry_revision_and_second_save_preserves_other_changes() {
    let fixture = Fixture::new();
    let mut draft = fixture.draft();
    draft.save(&fixture.coordinator).unwrap();
    let baseline = draft.baseline.clone();
    let baseline_revision = draft.baseline_revision;
    draft.edited.name = "My draft".into();
    let current = fixture.coordinator.current();
    let mut candidate = current.editable().as_ref().clone();
    candidate.settings.start_minimized = !candidate.settings.start_minimized;
    let intervening = fixture
        .coordinator
        .update(current.revision(), candidate)
        .unwrap();

    assert!(
        draft
            .save(&fixture.coordinator)
            .unwrap_err()
            .stale_actual_revision()
            .is_some()
    );
    assert_eq!(draft.baseline, baseline);
    assert_eq!(draft.baseline_revision, baseline_revision);
    assert_eq!(draft.next_attempt_revision, intervening.revision());
    assert_eq!(draft.edited.name, "My draft");
    assert!(!draft.refresh_if_clean(&intervening));
    let saved = draft.save(&fixture.coordinator).unwrap();
    assert_eq!(saved.editable().settings, intervening.editable().settings);
    assert_eq!(saved.editable().automations[0].name, "My draft");
    assert_eq!(draft.baseline_revision, saved.revision());
    assert!(!draft.is_dirty());
}

#[test]
fn returning_to_baseline_after_stale_failure_still_refreshes_from_publication() {
    let fixture = Fixture::new();
    let mut draft = fixture.draft();
    draft.save(&fixture.coordinator).unwrap();
    let original = draft.edited.clone();
    draft.edited.name = "Unsaved".into();
    let published = fixture.rename_durable("Latest");
    assert!(draft.save(&fixture.coordinator).is_err());
    draft.edited = original;
    assert!(draft.refresh_if_clean(&published));
    assert_eq!(draft.edited.name, "Latest");
}

#[test]
fn cancel_uses_latest_publication_and_deletion_handles_new_missing_and_stale_drafts() {
    let fixture = Fixture::new();
    let mut draft = fixture.draft();
    draft.save(&fixture.coordinator).unwrap();
    draft.edited.name = "Unsaved".into();
    let latest = fixture.rename_durable("Latest");
    assert!(draft.cancel(&latest));
    assert_eq!(draft.edited.name, "Latest");
    assert!(!draft.is_dirty());
    let intervening = fixture.rename_durable("Newer");
    assert!(draft.delete(&fixture.coordinator).is_err());
    assert_eq!(draft.next_attempt_revision, intervening.revision());
    let deleted = draft.delete(&fixture.coordinator).unwrap().unwrap();
    assert!(deleted.editable().automations.is_empty());
    assert!(!draft.cancel(&deleted));
}

#[test]
fn validation_failure_retains_creation_draft_and_publication() {
    let fixture = Fixture::new();
    let mut draft = fixture.draft();
    draft.edited.id.clear();
    let expected = draft.clone();
    let before = fixture.coordinator.current();
    assert!(draft.save(&fixture.coordinator).is_err());
    assert_eq!(draft, expected);
    assert!(draft.is_new() && draft.is_dirty());
    assert!(Arc::ptr_eq(&fixture.coordinator.current(), &before));
}
