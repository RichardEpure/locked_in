use std::{collections::HashSet, sync::Arc};

use dioxus::prelude::*;
use dioxus_icons::lucide::{ArrowDown, ArrowUp, Plus, Trash2};

use crate::{
    CAPTURE_TARGET_SIGNAL, cancel_capture,
    config::{Automation, AutomationCase, PublishedConfig},
};

use super::{action_editor::ActionEditor, matcher_group::MatcherGroup, mutations::add_action};

#[derive(Props, Clone, PartialEq)]
pub(super) struct CaseEditorProps {
    draft: Signal<Automation>,
    collapsed_matcher_groups: Signal<HashSet<(String, bool)>>,
    case_index: usize,
    case: AutomationCase,
    publication: Signal<Arc<PublishedConfig>>,
}

#[component]
pub(super) fn CaseEditor(props: CaseEditorProps) -> Element {
    let mut draft = props.draft;
    let collapsed_matcher_groups = props.collapsed_matcher_groups;
    let case_index = props.case_index;
    let case = props.case;
    let case_count = draft.read().cases.len();
    let priority = case_index + 1;
    rsx! {
        article { class: "case-card",
            header { class: "case-card__header",
                span { class: "priority", "{priority}" }
                input { class: "case-name", aria_label: "Case name", placeholder: "Case name", value: "{case.name}", oninput: move |event| draft.write().cases[case_index].name = event.value() }
                div { class: "toolbar tight",
                    button { class: "icon-button", aria_label: "Move case up", title: "Move up", disabled: case_index == 0, onclick: move |_| draft.write().cases.swap(case_index, case_index - 1), ArrowUp { size: 16, "aria-hidden": "true" } }
                    button { class: "icon-button", aria_label: "Move case down", title: "Move down", disabled: case_index + 1 == case_count, onclick: move |_| draft.write().cases.swap(case_index, case_index + 1), ArrowDown { size: 16, "aria-hidden": "true" } }
                    button { class: "icon-button danger", aria_label: "Delete case", title: "Delete case", onclick: move |_| {
                        let automation = draft.read();
                        let deleting_capture_target = CAPTURE_TARGET_SIGNAL.read().as_ref().is_some_and(|target|
                            target.automation_id == automation.id && target.case_id == automation.cases[case_index].id
                        );
                        drop(automation);
                        if deleting_capture_target {
                            cancel_capture();
                        }
                        draft.write().cases.remove(case_index);
                    }, Trash2 { size: 16, "aria-hidden": "true" } }
                }
            }
            div { class: "case-card__body",
                MatcherGroup { draft, collapsed_matcher_groups, case_index, case_id: case.id.clone(), case_name: case.name.clone(), exceptions: false, matchers: case.applications.clone() }
                MatcherGroup { draft, collapsed_matcher_groups, case_index, case_id: case.id.clone(), case_name: case.name.clone(), exceptions: true, matchers: case.exceptions.clone() }
                div { class: "actions-heading", div { h4 { "Send" } p { "One report per action, routed to selected devices" } }
                    button { class: "button secondary small", onclick: move |_| add_action(&mut draft, Some(case_index)), Plus { size: 15, "aria-hidden": "true" } "Add action" }
                }
                for (action_index, action) in case.actions.iter().cloned().enumerate() {
                    ActionEditor { key: "{action.id}", draft, case_index: Some(case_index), action_index, action, publication: props.publication }
                }
                if case.actions.is_empty() { div { class: "inline-empty compact", "No report actions configured." } }
            }
        }
    }
}
