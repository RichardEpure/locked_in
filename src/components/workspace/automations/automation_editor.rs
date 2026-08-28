use std::collections::HashSet;

use dioxus::prelude::*;

use crate::{
    CAPTURE_ARMED_SIGNAL, CAPTURE_TARGET_SIGNAL, CAPTURED_WINDOW_SIGNAL, CONFIG_SIGNAL,
    DIRTY_EDITOR_SIGNAL, UNSAVED_ENTITY_SIGNAL,
    automation_runtime::AutomationRuntime,
    cancel_capture,
    config::{self, Automation},
};

use super::{
    INVALID_REPORT_IDS,
    action_editor::ActionEditor,
    case_editor::CaseEditor,
    mutations::{
        add_action, add_case, insert_captured_matcher, matcher_group_name, reveal_last_matcher,
    },
};

#[derive(Props, Clone, PartialEq)]
pub(super) struct AutomationEditorProps {
    id: String,
    selected: Signal<Option<String>>,
    pending_delete: Signal<Option<String>>,
}

#[component]
pub(super) fn AutomationEditor(props: AutomationEditorProps) -> Element {
    let runtime = consume_context::<AutomationRuntime>();
    let id = props.id.clone();
    let mut selected = props.selected;
    let mut pending_delete = props.pending_delete;
    let original = CONFIG_SIGNAL
        .read()
        .automations
        .iter()
        .find(|item| item.id == id)
        .cloned()
        .unwrap_or_else(|| Automation {
            id: id.clone(),
            ..Automation::default()
        });
    let mut draft = use_signal(|| original.clone());
    let mut message = use_signal(|| None::<(bool, String)>);
    let mut collapsed_matcher_groups = use_signal(HashSet::<(String, bool)>::new);
    let snapshot = draft();
    let editor_token = format!("automation:{id}");
    let is_new = UNSAVED_ENTITY_SIGNAL.read().as_deref() == Some(editor_token.as_str());
    let dirty = snapshot != original || is_new;
    let invalid_report_ids = INVALID_REPORT_IDS.read();
    let has_invalid_report = snapshot
        .cases
        .iter()
        .flat_map(|case| &case.actions)
        .chain(&snapshot.otherwise_actions)
        .any(|action| invalid_report_ids.contains(&action.id));
    let capture_automation_id = id.clone();
    use_effect(move || {
        let Some(target) = CAPTURE_TARGET_SIGNAL.read().clone() else {
            return;
        };
        let Some(captured) = CAPTURED_WINDOW_SIGNAL.read().clone() else {
            return;
        };
        if target.automation_id != capture_automation_id {
            return;
        }
        let mut automation = draft.write();
        let Some((case_name, case_index)) = insert_captured_matcher(
            &mut automation,
            &target.case_id,
            target.exception,
            &captured,
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
    let effect_original = original.clone();
    use_effect(move || {
        let pending = UNSAVED_ENTITY_SIGNAL.read().as_deref() == Some(effect_token.as_str());
        if draft() != effect_original || pending {
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
    let cancel_original = original.clone();
    let cancel_id = id.clone();
    let cancel_token = editor_token.clone();
    let delete_runtime = runtime.clone();
    let save_runtime = runtime.clone();

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
                        let mut config = CONFIG_SIGNAL.read().clone();
                        let mut copy = draft();
                        copy.id = config.next_id(&format!("{}-copy", copy.id));
                        copy.name = format!("{} Copy", copy.name);
                        copy.enabled = false;
                        let copy_id = copy.id.clone();
                        config.automations.push(copy);
                        *CONFIG_SIGNAL.write() = config;
                        *UNSAVED_ENTITY_SIGNAL.write() = Some(format!("automation:{copy_id}"));
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
                                let mut next = CONFIG_SIGNAL.read().clone();
                                next.automations.retain(|item| item.id != id);
                                 match config::save(&next) {
                                    Ok(()) => { delete_runtime.replace_config(next.clone()).expect("saved configuration must compile"); *CONFIG_SIGNAL.write() = next; *DIRTY_EDITOR_SIGNAL.write() = None; *UNSAVED_ENTITY_SIGNAL.write() = None; pending_delete.set(None); selected.set(None); }
                                    Err(error) => message.set(Some((false, format!("Delete failed: {error}")))),
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
                    CaseEditor { key: "{case.id}", draft, collapsed_matcher_groups, case_index, case }
                }
            }
            section { class: "editor-card otherwise-card",
                div { class: "section-heading split", span { class: "step muted", "ELSE" } div { h3 { "Otherwise" } p { "Runs only when no case matches" } }
                    button { class: "button secondary", onclick: move |_| add_action(&mut draft, None), "+ Add action" }
                }
                for (action_index, action) in snapshot.otherwise_actions.iter().cloned().enumerate() {
                    ActionEditor { key: "{action.id}", draft, case_index: None, action_index, action }
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
                    if UNSAVED_ENTITY_SIGNAL.read().as_deref() == Some(cancel_token.as_str()) {
                        CONFIG_SIGNAL.write().automations.retain(|automation| automation.id != cancel_id);
                        *UNSAVED_ENTITY_SIGNAL.write() = None;
                        *DIRTY_EDITOR_SIGNAL.write() = None;
                        selected.set(None);
                    } else {
                        draft.set(cancel_original.clone());
                        message.set(None);
                    }
                }, "Cancel" }
                button {
                    class: "button primary",
                    disabled: !dirty || has_invalid_report,
                    onclick: move |_| {
                        let mut next = CONFIG_SIGNAL.read().clone();
                        if let Some(index) = next.automations.iter().position(|item| item.id == props.id) {
                            next.automations[index] = draft();
                        }
                        let errors = next.validate();
                        if errors.is_empty() {
                            match config::save(&next) {
                                Ok(()) => {
                                    save_runtime
                                        .replace_config(next.clone())
                                        .expect("saved configuration must compile");
                                    *CONFIG_SIGNAL.write() = next;
                                    *UNSAVED_ENTITY_SIGNAL.write() = None;
                                    *DIRTY_EDITOR_SIGNAL.write() = None;
                                    message.set(Some((true, "Automation saved".into())));
                                }
                                Err(error) => message.set(Some((false, format!("Save failed: {error}")))),
                            }
                        } else {
                            let text = errors.iter().take(3).map(|error| error.message.as_str()).collect::<Vec<_>>().join("; ");
                            message.set(Some((false, text)));
                        }
                    },
                    "Save automation"
                }
            }
        }
    }
}
