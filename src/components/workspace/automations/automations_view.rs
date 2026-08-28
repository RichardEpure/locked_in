use dioxus::prelude::*;

use crate::{
    CAPTURE_TARGET_SIGNAL, CONFIG_REVISION_SIGNAL, CONFIG_SIGNAL, DIRTY_EDITOR_SIGNAL,
    UNSAVED_ENTITY_SIGNAL, config::Automation,
};

use crate::components::workspace::empty_state::EmptyState;
use crate::components::workspace::selection::SelectionProps;

use super::automation_editor::AutomationEditor;

#[component]
pub(in crate::components::workspace) fn AutomationsView(props: SelectionProps) -> Element {
    let mut selected = props.selected;
    let mut query = use_signal(String::new);
    let pending_delete = use_signal(|| None::<String>);
    let automations = CONFIG_SIGNAL.read().automations.clone();
    let normalized_query = query().to_lowercase();
    let revision = *CONFIG_REVISION_SIGNAL.read();
    let navigation_locked =
        DIRTY_EDITOR_SIGNAL.read().is_some() || CAPTURE_TARGET_SIGNAL.read().is_some();

    rsx! {
        aside {
            class: "entity-list",
            header {
                div { h1 { "Automations" } p { "Run actions when windows match" } }
                button {
                    class: "icon-button primary",
                    aria_label: "New automation",
                    title: "New automation (Ctrl+N)",
                    disabled: navigation_locked,
                    onclick: move |_| {
                        let mut config = CONFIG_SIGNAL.read().clone();
                        let id = config.next_id("automation");
                        config.automations.push(Automation { id: id.clone(), ..Automation::default() });
                        *CONFIG_SIGNAL.write() = config;
                        *UNSAVED_ENTITY_SIGNAL.write() = Some(format!("automation:{id}"));
                        selected.set(Some(id));
                    },
                    "+"
                }
            }
            input {
                class: "search",
                placeholder: "Search automations",
                value: "{query}",
                oninput: move |event| query.set(event.value()),
            }
            div {
                class: "entity-list__items",
                for automation in automations.iter().filter(|item| item.name.to_lowercase().contains(&normalized_query)).cloned() {
                    button {
                        key: "{automation.id}",
                        class: if selected().as_deref() == Some(&automation.id) { "entity-row selected" } else { "entity-row" },
                        disabled: navigation_locked && selected().as_deref() != Some(&automation.id),
                        onclick: {
                            let id = automation.id.clone();
                            move |_| selected.set(Some(id.clone()))
                        },
                        span { class: if automation.enabled { "status-dot online" } else { "status-dot" } }
                        span { class: "entity-row__copy", strong { "{automation.name}" } small { "{automation.cases.len()} cases" } }
                    }
                }
            }
        }
        section {
            class: "workspace",
            if let Some(id) = selected().filter(|id| automations.iter().any(|automation| automation.id == *id)) {
                AutomationEditor { key: "{id}-{revision}", id, selected, pending_delete }
            } else {
                EmptyState { title: "Select an automation", copy: "Create or select an automation to configure its event, ordered cases, and report routes." }
            }
        }
    }
}
