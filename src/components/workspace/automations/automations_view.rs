use dioxus::prelude::*;
use dioxus_icons::lucide::{Plus, Search};

use crate::{DIRTY_EDITOR_SIGNAL, components::capture, config::Automation};

use crate::components::workspace::empty_state::EmptyState;
use crate::components::workspace::selection::SelectionProps;

use super::{automation_editor::AutomationEditor, publication::new_automation};

#[component]
pub(in crate::components::workspace) fn AutomationsView(props: SelectionProps) -> Element {
    let mut selected = props.selected;
    let mut query = use_signal(String::new);
    let pending_delete = use_signal(|| None::<String>);
    let mut pending_draft = use_signal(|| None::<Automation>);
    let publication = consume_context::<crate::components::PublishedConfigContext>();
    let published = publication.required();
    let mut automations = published.editable().automations.clone();
    if let Some(pending) = pending_draft.read().clone()
        && !automations
            .iter()
            .any(|automation| automation.id == pending.id)
    {
        automations.push(pending);
    }
    let normalized_query = query().to_lowercase();
    let session = capture::session();
    let navigation_locked = DIRTY_EDITOR_SIGNAL.read().is_some() || session.target().is_some();

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
                        let automation = new_automation(&publication.required());
                        let id = automation.id.clone();
                        pending_draft.set(Some(automation));
                        let token = format!("automation:{id}");
                        *DIRTY_EDITOR_SIGNAL.write() = Some(token);
                        selected.set(Some(id));
                    },
                    Plus { size: 16, "aria-hidden": "true" }
                }
            }
            div {
                class: "search-field",
                Search { size: 15, "aria-hidden": "true" }
                input {
                    class: "search",
                    placeholder: "Search automations",
                    value: "{query}",
                    oninput: move |event| query.set(event.value()),
                }
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
                AutomationEditor { key: "{id}", id, selected, pending_delete, pending_draft }
            } else {
                EmptyState { title: "Select an automation", copy: session.cancellation_message().unwrap_or("Create or select an automation to configure its event, ordered cases, and report routes.") }
            }
        }
    }
}
