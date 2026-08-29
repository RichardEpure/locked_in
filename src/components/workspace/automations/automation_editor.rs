use std::{collections::HashSet, sync::Arc};

use dioxus::prelude::*;

use crate::{
    CAPTURE_ARMED_SIGNAL, CAPTURE_GENERATION_SIGNAL, CAPTURE_TARGET_SIGNAL, CAPTURED_WINDOW_SIGNAL,
    DIRTY_EDITOR_SIGNAL, UNSAVED_ENTITY_SIGNAL, cancel_capture,
    config::{Automation, ConfigCoordinator, PublishedConfig},
};

use super::{
    INVALID_REPORT_IDS,
    action_editor::ActionEditor,
    case_editor::CaseEditor,
    mutations::{
        add_action, add_case, insert_captured_matcher, matcher_group_name, reveal_last_matcher,
    },
    publication::{
        AutomationCommitError, cancel_automation, delete_automation, duplicate_automation,
        save_automation,
    },
};

#[derive(Props, Clone, PartialEq)]
pub(super) struct AutomationEditorProps {
    id: String,
    selected: Signal<Option<String>>,
    pending_delete: Signal<Option<String>>,
    pending_draft: Signal<Option<Automation>>,
    publication: Signal<Arc<PublishedConfig>>,
}

#[component]
pub(super) fn AutomationEditor(props: AutomationEditorProps) -> Element {
    let coordinator = consume_context::<Option<Arc<ConfigCoordinator>>>()
        .expect("configuration coordinator is available after bootstrap");
    let id = props.id.clone();
    let mut selected = props.selected;
    let mut pending_delete = props.pending_delete;
    let mut pending_draft = props.pending_draft;
    let mut publication = props.publication;
    let published = publication.read().clone();
    let local_draft = pending_draft
        .read()
        .as_ref()
        .filter(|automation| automation.id == id)
        .cloned();
    let is_new = local_draft.is_some();
    let original = local_draft
        .or_else(|| {
            published
                .editable()
                .automations
                .iter()
                .find(|item| item.id == id)
                .cloned()
        })
        .unwrap_or_else(|| Automation {
            id: id.clone(),
            ..Automation::default()
        });
    let mut draft = use_signal(|| original.clone());
    let initial_revision = published.revision();
    let mut base = use_signal(|| original.clone());
    let mut base_revision = use_signal(move || initial_revision);
    let mut expected_revision = use_signal(move || initial_revision);
    let mut message = use_signal(|| None::<(bool, String)>);
    let mut collapsed_matcher_groups = use_signal(HashSet::<(String, bool)>::new);
    let snapshot = draft();
    let editor_token = format!("automation:{id}");
    let dirty = snapshot != base() || is_new;
    let invalid_report_ids = INVALID_REPORT_IDS.read();
    let has_invalid_report = snapshot
        .cases
        .iter()
        .flat_map(|case| &case.actions)
        .chain(&snapshot.otherwise_actions)
        .any(|action| invalid_report_ids.contains(&action.id));
    let capture_automation_id = id.clone();
    use_effect(move || {
        let generation = *CAPTURE_GENERATION_SIGNAL.read();
        let Some(target) = CAPTURE_TARGET_SIGNAL.read().clone() else {
            return;
        };
        let Some(captured) = CAPTURED_WINDOW_SIGNAL.read().clone() else {
            return;
        };
        if target.automation_id != capture_automation_id
            || !captured.belongs_to(generation, &Some(target.clone()))
        {
            return;
        }
        let mut automation = draft.write();
        let Some((case_name, case_index)) = insert_captured_matcher(
            &mut automation,
            &target.case_id,
            target.exception,
            &captured.window,
        ) else {
            drop(automation);
            cancel_capture();
            message.set(Some((
                false,
                "Capture cancelled because the target case no longer exists".into(),
            )));
            return;
        };
        drop(automation);
        collapsed_matcher_groups
            .write()
            .remove(&(target.case_id, target.exception));
        reveal_last_matcher(case_index, target.exception);
        cancel_capture();
        message.set(Some((
            true,
            format!(
                "Captured window added to \"{case_name}\" -> {}",
                matcher_group_name(target.exception)
            ),
        )));
    });
    let sync_id = id.clone();
    use_effect(move || {
        let published = publication.read().clone();
        let pending = pending_draft
            .read()
            .as_ref()
            .is_some_and(|automation| automation.id == sync_id);
        let base_snapshot = base();
        let draft_snapshot = draft();
        let Some(durable) = clean_editor_publication(
            published.revision(),
            &published.editable().automations,
            &sync_id,
            pending,
            base_revision(),
            &base_snapshot,
            &draft_snapshot,
        ) else {
            return;
        };
        base.set(durable.clone());
        draft.set(durable);
        base_revision.set(published.revision());
        expected_revision.set(published.revision());
    });
    let feedback_automation_id = id.clone();
    use_effect(move || {
        if *CAPTURE_ARMED_SIGNAL.read()
            && CAPTURE_TARGET_SIGNAL
                .read()
                .as_ref()
                .is_some_and(|target| target.automation_id == feedback_automation_id)
        {
            message.set(None);
        }
    });
    let effect_token = editor_token.clone();
    use_effect(move || {
        let pending = UNSAVED_ENTITY_SIGNAL.read().as_deref() == Some(effect_token.as_str());
        if draft() != base() || pending {
            *DIRTY_EDITOR_SIGNAL.write() = Some(effect_token.clone());
        } else if DIRTY_EDITOR_SIGNAL.read().as_deref() == Some(effect_token.as_str()) {
            *DIRTY_EDITOR_SIGNAL.write() = None;
        }
    });
    let cleanup_token = editor_token.clone();
    let cleanup_id = id.clone();
    use_drop(move || {
        if DIRTY_EDITOR_SIGNAL.read().as_deref() == Some(cleanup_token.as_str()) {
            *DIRTY_EDITOR_SIGNAL.write() = None;
        }
        if CAPTURE_TARGET_SIGNAL
            .read()
            .as_ref()
            .is_some_and(|target| target.automation_id == cleanup_id)
        {
            cancel_capture();
        }
    });
    let cancel_id = id.clone();
    let duplicate_publication = publication;
    let delete_coordinator = coordinator.clone();
    let cancel_coordinator = coordinator.clone();
    let save_coordinator = coordinator;

    rsx! {
        header {
            class: "workspace-header",
            div {
                div { class: "eyebrow", "FOCUSED WINDOW AUTOMATION" }
                h2 { "{snapshot.name}" if dirty { span { class: "dirty-dot", title: "Unsaved changes", "•" } } }
                p { "First matching case runs. Otherwise is used only when no case matches." }
            }
            div { class: "toolbar",
                button {
                    class: "button ghost",
                    disabled: dirty,
                    onclick: move |_| {
                        let copy = duplicate_automation(&duplicate_publication.read(), &draft());
                        let copy_id = copy.id.clone();
                        pending_draft.set(Some(copy));
                        let token = format!("automation:{copy_id}");
                        *UNSAVED_ENTITY_SIGNAL.write() = Some(token.clone());
                        *DIRTY_EDITOR_SIGNAL.write() = Some(token);
                        selected.set(Some(copy_id));
                    },
                    "Duplicate"
                }
                button {
                    class: "button danger-ghost",
                    onclick: {
                        let id = props.id.clone();
                        move |_| {
                            if pending_delete().as_deref() == Some(&id) {
                                if is_new {
                                    pending_draft.set(None);
                                    *DIRTY_EDITOR_SIGNAL.write() = None;
                                    *UNSAVED_ENTITY_SIGNAL.write() = None;
                                    pending_delete.set(None);
                                    selected.set(None);
                                    return;
                                }
                                match delete_automation(&delete_coordinator, expected_revision(), &id) {
                                    Ok(published) => {
                                        expected_revision.set(published.revision());
                                        publication.set(published);
                                        *DIRTY_EDITOR_SIGNAL.write() = None;
                                        *UNSAVED_ENTITY_SIGNAL.write() = None;
                                        pending_delete.set(None);
                                        selected.set(None);
                                    }
                                    Err(error) => {
                                        message.set(Some((false, commit_error("Delete", &error, &mut expected_revision))));
                                    }
                                }
                            } else {
                                pending_delete.set(Some(id.clone()));
                                message.set(Some((false, "Click Delete again to confirm".into())));
                            }
                        }
                    },
                    "Delete"
                }
            }
        }
        div {
            class: "editor-scroll",
            section { class: "editor-card overview-card",
                div { class: "section-heading", span { class: "step", "01" } div { h3 { "Automation" } p { "Name this automation and choose when it is active" } } }
                div { class: "form-grid two",
                    label { "Name" input { value: "{snapshot.name}", oninput: move |event| draft.write().name = event.value() } }
                    label { class: "toggle-field", span { "Enabled" } input { type: "checkbox", checked: snapshot.enabled, onchange: move |event| draft.write().enabled = event.checked() } small { "Leave this off to save an incomplete draft; turn it on when ready to run." } }
                }
            }
            section { class: "editor-card trigger-card",
                div { class: "section-heading", span { class: "step", "02" } div { h3 { "When" } p { "This automation runs when focus moves to another window" } } }
                div { class: "trigger-summary", span { class: "trigger-icon", "W" } div { strong { "Focused window changes" } small { "Match title, class, and executable details in the cases below" } } span { class: "pill", "Windows" } }
            }
            section { class: "editor-card",
                div { class: "section-heading split", span { class: "step", "03" } div { h3 { "Cases" } p { "Evaluated from top to bottom; first match wins" } }
                    button { class: "button secondary", onclick: move |_| add_case(&mut draft), "+ Add case" }
                }
                if snapshot.cases.is_empty() {
                    div { class: "inline-empty", "No cases yet. Add a case or use an Otherwise action." }
                }
                for (case_index, case) in snapshot.cases.iter().cloned().enumerate() {
                    CaseEditor { key: "{case.id}", draft, collapsed_matcher_groups, case_index, case, publication }
                }
            }
            section { class: "editor-card otherwise-card",
                div { class: "section-heading split", span { class: "step muted", "ELSE" } div { h3 { "Otherwise" } p { "Runs only when no case matches" } }
                    button { class: "button secondary", onclick: move |_| add_action(&mut draft, None), "+ Add action" }
                }
                for (action_index, action) in snapshot.otherwise_actions.iter().cloned().enumerate() {
                    ActionEditor { key: "{action.id}", draft, case_index: None, action_index, action, publication }
                }
                if snapshot.otherwise_actions.is_empty() { div { class: "inline-empty compact", "Optional. Leave empty to do nothing when no cases match." } }
            }
        }
        footer { class: "save-bar",
            div { class: "save-bar__status",
                if has_invalid_report { span { class: "message error", role: "status", aria_live: "polite", "Complete or correct every hexadecimal report before saving" } }
                if let Some((success, text)) = message() { span { class: if success { "message success" } else { "message error" }, role: "status", aria_live: "polite", "{text}" } }
            }
            div { class: "toolbar",
                button { class: "button ghost", disabled: !dirty, onclick: move |_| {
                    let current = cancel_coordinator.current();
                    if let Some(durable) = cancel_automation(&current, &cancel_id, is_new) {
                        base.set(durable.clone());
                        draft.set(durable);
                        base_revision.set(current.revision());
                        expected_revision.set(current.revision());
                        publication.set(current);
                        *DIRTY_EDITOR_SIGNAL.write() = None;
                        message.set(None);
                    } else {
                        pending_draft.set(None);
                        *UNSAVED_ENTITY_SIGNAL.write() = None;
                        *DIRTY_EDITOR_SIGNAL.write() = None;
                        selected.set(None);
                    }
                }, "Cancel" }
                button {
                    class: "button primary",
                    disabled: !dirty || has_invalid_report,
                    onclick: move |_| {
                        let draft_snapshot = draft();
                        match save_automation(&save_coordinator, expected_revision(), &draft_snapshot, is_new) {
                            Ok(published) => {
                                expected_revision.set(published.revision());
                                if let Some(saved) = published.editable().automations.iter().find(|automation| automation.id == props.id).cloned() {
                                    base.set(saved.clone());
                                    draft.set(saved);
                                }
                                base_revision.set(published.revision());
                                publication.set(published);
                                pending_draft.set(None);
                                *UNSAVED_ENTITY_SIGNAL.write() = None;
                                *DIRTY_EDITOR_SIGNAL.write() = None;
                                message.set(Some((true, "Automation saved".into())));
                            }
                            Err(error) => message.set(Some((false, commit_error("Save", &error, &mut expected_revision)))),
                        }
                    },
                    "Save automation"
                }
            }
        }
    }
}

fn clean_editor_publication(
    published_revision: u64,
    published_automations: &[Automation],
    automation_id: &str,
    is_new: bool,
    base_revision: u64,
    base: &Automation,
    draft: &Automation,
) -> Option<Automation> {
    if is_new || published_revision == base_revision || draft != base {
        return None;
    }
    published_automations
        .iter()
        .find(|automation| automation.id == automation_id)
        .cloned()
}

fn commit_error(
    operation: &str,
    error: &AutomationCommitError,
    expected_revision: &mut Signal<u64>,
) -> String {
    if let Some(actual) = error.stale_actual_revision() {
        expected_revision.set(actual);
        return format!(
            "{operation} failed because the configuration changed. Your draft is preserved; review it and save again, or cancel to restore the published version"
        );
    }
    format!("{operation} failed; your draft is preserved: {error}")
}

#[cfg(test)]
mod tests;
