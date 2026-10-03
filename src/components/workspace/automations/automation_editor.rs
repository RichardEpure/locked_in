use std::{collections::HashSet, sync::Arc};

use dioxus::prelude::*;
use dioxus_icons::lucide::{AppWindow, Plus};

use crate::{
    DIRTY_EDITOR_SIGNAL,
    components::{PublishedConfigContext, capture},
    config::{Automation, AutomationCase, ConfigCoordinator},
};

use super::{
    INVALID_REPORT_IDS,
    action_editor::ActionEditor,
    case_editor::CaseEditor,
    draft::AutomationDraft,
    editing::{AutomationCommitError, duplicate_automation},
    editor_helpers::{
        add_action, insert_captured_matcher, matcher_group_name, next_child_id, reveal_last_matcher,
    },
};

#[derive(Props, Clone, PartialEq)]
pub(super) struct AutomationEditorProps {
    id: String,
    selected: Signal<Option<String>>,
    pending_delete: Signal<Option<String>>,
    pending_draft: Signal<Option<Automation>>,
}

fn add_case(draft: &mut Signal<AutomationDraft>) {
    let snapshot = draft.read();
    let index = snapshot.edited.cases.len() + 1;
    let id = next_child_id(
        "case",
        snapshot.edited.cases.iter().map(|case| case.id.as_str()),
    );
    drop(snapshot);
    draft.write().edited.cases.push(AutomationCase {
        id,
        name: format!("Case {index}"),
        ..AutomationCase::default()
    });
}

#[component]
pub(super) fn AutomationEditor(props: AutomationEditorProps) -> Element {
    let coordinator = consume_context::<Option<Arc<ConfigCoordinator>>>()
        .expect("configuration coordinator is available after bootstrap");
    let id = props.id.clone();
    let mut selected = props.selected;
    let mut pending_delete = props.pending_delete;
    let mut pending_draft = props.pending_draft;
    let publication = consume_context::<PublishedConfigContext>();
    let published = publication.required();
    let local_draft = pending_draft
        .read()
        .as_ref()
        .filter(|automation| automation.id == id)
        .cloned();
    let initial_is_new = local_draft.is_some();
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
    let mut draft = use_signal(|| {
        if initial_is_new {
            AutomationDraft::create(published.revision(), original)
        } else {
            AutomationDraft::edit(published.revision(), original)
        }
    });
    let mut message = use_signal(|| None::<(bool, String)>);
    let mut collapsed_matcher_groups = use_signal(HashSet::<(String, bool)>::new);
    let snapshot = draft.read().edited.clone();
    let editor_token = format!("automation:{id}");
    let dirty = draft.read().is_dirty();
    let capture_notice = capture::session().cancellation_message();
    let invalid_report_ids = INVALID_REPORT_IDS.read();
    let has_invalid_report = snapshot
        .cases
        .iter()
        .flat_map(|case| &case.actions)
        .chain(&snapshot.otherwise_actions)
        .any(|action| invalid_report_ids.contains(&action.id));
    let capture_automation_id = id.clone();
    use_effect(move || {
        let session = capture::session();
        let Some(target) = session
            .target()
            .filter(|target| target.automation_id == capture_automation_id)
            .cloned()
        else {
            return;
        };
        if !draft
            .read()
            .edited
            .cases
            .iter()
            .any(|case| case.id == target.case_id)
        {
            capture::target_removed(session.generation());
            return;
        }
        if session.captured().is_none() {
            return;
        }
        let Some(window) = capture::take_targeted(session.generation(), &target) else {
            return;
        };
        let mut automation = draft.write();
        let Some((case_name, case_index)) = insert_captured_matcher(
            &mut automation.edited,
            &target.case_id,
            target.exception,
            &window,
        ) else {
            drop(automation);
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
        message.set(Some((
            true,
            format!(
                "Captured window added to \"{case_name}\" -> {}",
                matcher_group_name(target.exception)
            ),
        )));
    });
    use_effect(move || {
        let published = publication.required();
        let mut refreshed = draft();
        if refreshed.refresh_if_clean(&published) {
            draft.set(refreshed);
        }
    });
    let feedback_automation_id = id.clone();
    use_effect(move || {
        let session = capture::session();
        if session.is_armed()
            && session
                .target()
                .is_some_and(|target| target.automation_id == feedback_automation_id)
        {
            message.set(None);
        }
    });
    let effect_token = editor_token.clone();
    use_effect(move || {
        if draft.read().is_dirty() {
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
        let session = capture::session();
        if session
            .target()
            .is_some_and(|target| target.automation_id == cleanup_id)
        {
            if !draft.peek().is_new()
                && !publication
                    .required()
                    .editable()
                    .automations
                    .iter()
                    .any(|automation| automation.id == cleanup_id)
            {
                capture::target_removed(session.generation());
            } else {
                capture::cancel(session.generation());
            }
        }
    });
    let duplicate_publication = publication;
    let delete_coordinator = coordinator.clone();
    let cancel_coordinator = coordinator.clone();
    let save_coordinator = coordinator;

    rsx! {
        header {
            class: "workspace-header",
            div {
                div { class: "eyebrow", "FOCUSED WINDOW AUTOMATION" }
                h2 { "{snapshot.name}" if dirty { span { class: "dirty-dot", title: "Unsaved changes", aria_hidden: true, "•" } span { class: "visually-hidden", "Unsaved changes" } } }
                p { "First matching case runs. Otherwise is used only when no case matches." }
            }
            div { class: "toolbar",
                button {
                    class: "button ghost",
                    disabled: dirty,
                    onclick: move |_| {
                        let copy = duplicate_automation(&duplicate_publication.required(), &draft.read().edited);
                        let copy_id = copy.id.clone();
                        pending_draft.set(Some(copy));
                        let token = format!("automation:{copy_id}");
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
                                let result = draft.write().delete(&delete_coordinator);
                                match result {
                                    Ok(published) => {
                                        if let Some(published) = published { publication.acknowledge(published); }
                                        pending_draft.set(None);
                                        *DIRTY_EDITOR_SIGNAL.write() = None;
                                        pending_delete.set(None);
                                        selected.set(None);
                                    }
                                    Err(error) => {
                                        message.set(Some((false, commit_error("Delete", &error))));
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
            section { class: "editor-card",
                div { class: "section-heading split", span { class: "step", "01" } div { h3 { "Automation" } p { "Name this automation and choose when it is active" } }
                    label { class: "toggle-field", span { "Enabled" } input { type: "checkbox", checked: snapshot.enabled, onchange: move |event| draft.write().edited.enabled = event.checked() } }
                }
                div { class: "form-grid two",
                    label { "Name" input { value: "{snapshot.name}", oninput: move |event| draft.write().edited.name = event.value() } }
                }
            }
            section { class: "editor-card",
                div { class: "section-heading", span { class: "step", "02" } div { h3 { "When" } p { "This automation runs when focus moves to another window" } } }
                div { class: "trigger-summary", span { class: "trigger-icon", AppWindow { size: 18, "aria-hidden": "true" } } div { strong { "Focused window changes" } small { "Match title, class, and executable details in the cases below" } } span { class: "pill", "Windows" } }
            }
            section { class: "editor-card",
                div { class: "section-heading split", span { class: "step", "03" } div { h3 { "Cases" } p { "Evaluated from top to bottom; first match wins" } }
                    button { class: "button secondary", onclick: move |_| add_case(&mut draft), Plus { size: 16, "aria-hidden": "true" } "Add case" }
                }
                if snapshot.cases.is_empty() {
                    div { class: "inline-empty", "No cases yet. Add a case or use an Otherwise action." }
                }
                for (case_index, case) in snapshot.cases.iter().cloned().enumerate() {
                    CaseEditor { key: "{case.id}", draft, collapsed_matcher_groups, case_index, case }
                }
            }
            section { class: "editor-card otherwise-card",
                div { class: "section-heading split", span { class: "step muted", "ELSE" } div { h3 { "Otherwise" } p { "Runs only when no case matches" } }
                    button { class: "button secondary", onclick: move |_| add_action(&mut draft, None), Plus { size: 16, "aria-hidden": "true" } "Add action" }
                }
                for (action_index, action) in snapshot.otherwise_actions.iter().cloned().enumerate() {
                    ActionEditor { key: "{action.id}", draft, case_index: None, action_index, action }
                }
                if snapshot.otherwise_actions.is_empty() { div { class: "inline-empty compact", "Optional. Leave empty to do nothing when no cases match." } }
            }
        }
        footer { class: "save-bar",
            div { class: "save-bar__status",
                if let Some(text) = capture_notice { span { class: "message error", role: "status", aria_live: "polite", "{text}" } }
                if has_invalid_report { span { class: "message error", role: "status", aria_live: "polite", "Complete or correct every hexadecimal report before saving" } }
                if let Some((success, text)) = message() { span { class: if success { "message success" } else { "message error" }, role: "status", aria_live: "polite", "{text}" } }
            }
            div { class: "toolbar",
                button { class: "button ghost", disabled: !dirty, onclick: move |_| {
                    let current = cancel_coordinator.current();
                    let restored = draft.write().cancel(&current);
                    if restored {
                        publication.acknowledge(current);
                        *DIRTY_EDITOR_SIGNAL.write() = None;
                        message.set(None);
                    } else {
                        pending_draft.set(None);
                        *DIRTY_EDITOR_SIGNAL.write() = None;
                        selected.set(None);
                    }
                }, "Cancel" }
                button {
                    class: "button primary",
                    disabled: !dirty || has_invalid_report,
                    onclick: move |_| {
                        let result = draft.write().save(&save_coordinator);
                        match result {
                            Ok(published) => {
                                publication.acknowledge(published);
                                pending_draft.set(None);
                                *DIRTY_EDITOR_SIGNAL.write() = None;
                                message.set(Some((true, "Automation saved".into())));
                            }
                            Err(error) => message.set(Some((false, commit_error("Save", &error)))),
                        }
                    },
                    "Save automation"
                }
            }
        }
    }
}

fn commit_error(operation: &str, error: &AutomationCommitError) -> String {
    if error.stale_actual_revision().is_some() {
        return format!(
            "{operation} failed because the configuration changed. Your draft is preserved; review it and save again, or cancel to restore the published version"
        );
    }
    format!("{operation} failed; your draft is preserved: {error}")
}
