use dioxus::prelude::*;
use dioxus_icons::lucide::{ChevronDown, ChevronRight, Plus, Search};

use crate::{
    CAPTURE_TARGET_SIGNAL, DIRTY_EDITOR_SIGNAL, UNSAVED_ENTITY_SIGNAL,
    automation_runtime::AutomationRuntime,
    config::Device,
    hid::{HidInventory, HidRefreshState},
};

use super::{
    device_editor::DeviceEditor,
    discovery_row::DiscoveryRow,
    draft::{DeviceDraft, clear_published_pending},
};
use crate::components::workspace::{
    empty_state::EmptyState,
    hid_inventory::{HidInventoryContext, hid_presence_view},
    published_config::PublishedConfigContext,
    selection::SelectionProps,
};

fn request_inventory_refresh(
    runtime: &AutomationRuntime,
    inventory: &HidInventory,
    mut queued_revision: Signal<Option<u64>>,
    mut request_error: Signal<Option<String>>,
) {
    if inventory.refresh_state == HidRefreshState::Refreshing {
        queued_revision.set(None);
    } else {
        queued_revision.set(Some(inventory.revision));
    }
    match runtime.request_hid_refresh() {
        Ok(_) => request_error.set(None),
        Err(error) => {
            queued_revision.set(None);
            request_error.set(Some(error.to_string()));
        }
    }
}

#[component]
pub(in crate::components::workspace) fn DevicesView(props: SelectionProps) -> Element {
    let runtime = consume_context::<AutomationRuntime>();
    let inventory_context = consume_context::<HidInventoryContext>();
    let publication_context = consume_context::<PublishedConfigContext>();
    let published = publication_context.current();
    let inventory = inventory_context.current();
    let mut selected = props.selected;
    let mut pending_draft = use_signal(|| None::<DeviceDraft>);
    let mut query = use_signal(String::new);
    let mut discovery_open = use_signal(|| false);
    let queued_revision = use_signal(|| None::<u64>);
    let request_error = use_signal(|| None::<String>);
    let mount_inventory = inventory.clone();
    let mount_runtime = runtime.clone();
    use_effect(move || {
        request_inventory_refresh(
            &mount_runtime,
            &mount_inventory,
            queued_revision,
            request_error,
        );
    });

    let devices = published.editable().devices.clone();
    let physical_count = inventory
        .rows
        .iter()
        .map(|row| row.match_count)
        .sum::<usize>();
    let normalized_query = query().to_lowercase();
    let filtered_devices = devices
        .iter()
        .filter(|device| device.name.to_lowercase().contains(&normalized_query))
        .cloned()
        .map(|device| {
            let presence = hid_presence_view(inventory.presence(&device));
            (device, presence)
        })
        .collect::<Vec<_>>();
    let is_refreshing = inventory.refresh_state == HidRefreshState::Refreshing;
    let is_queued = !is_refreshing && queued_revision() == Some(inventory.revision);
    let refresh_pending = is_refreshing || is_queued;
    let (status_class, status_text) = if let Some(error) = request_error() {
        (
            "discovery-status error",
            format!("Refresh request failed: {error}"),
        )
    } else if is_refreshing {
        (
            "discovery-status",
            "Refreshing connected interfaces...".to_string(),
        )
    } else if is_queued {
        ("discovery-status", "Refresh queued".to_string())
    } else {
        match &inventory.refresh_state {
            HidRefreshState::NotAttempted => (
                "discovery-status",
                "Connected interfaces have not been refreshed".to_string(),
            ),
            HidRefreshState::Ready => (
                "discovery-status success",
                format!("Refresh complete: {physical_count} found"),
            ),
            HidRefreshState::Failed { error } => (
                "discovery-status error",
                format!("Refresh failed: {error}. Retained rows are stale."),
            ),
            HidRefreshState::Refreshing => unreachable!(),
        }
    };
    let refresh_summary = if is_refreshing {
        "Refreshing...".to_string()
    } else if is_queued {
        "Queued".to_string()
    } else {
        match inventory.refresh_state {
            HidRefreshState::Ready => format!("{physical_count} found · Refresh"),
            HidRefreshState::Failed { .. } => format!("{physical_count} stale · Refresh"),
            HidRefreshState::NotAttempted => "Refresh".to_string(),
            HidRefreshState::Refreshing => unreachable!(),
        }
    };
    let navigation_locked =
        DIRTY_EDITOR_SIGNAL.read().is_some() || CAPTURE_TARGET_SIGNAL.read().is_some();
    let create_config = published.editable().clone();
    let create_revision = published.revision();
    use_effect(move || {
        let publication = publication_context.current();
        let mut pending = pending_draft();
        if clear_published_pending(&mut pending, &publication) {
            pending_draft.set(pending);
        }
    });
    use_effect(move || {
        let selected_id = selected();
        let is_open = discovery_open();
        if selected_id.is_some() && !is_open {
            spawn(async move {
                let _ = document::eval(
                    "requestAnimationFrame(() => document.querySelector('.entity-list__items .entity-row.selected')?.scrollIntoView({ block: 'nearest' }))",
                )
                .await;
            });
        }
    });
    rsx! {
        aside { class: "entity-list",
            header { div { h1 { "Devices" } p { "HID destinations for report actions" } }
                button { class: "icon-button primary", aria_label: "New device", disabled: navigation_locked, onclick: move |_| {
                    let id = create_config.next_id("device");
                    pending_draft.set(Some(DeviceDraft::create(
                        create_revision,
                        Device { id: id.clone(), name: "New device".into(), report_length: 32, ..Device::default() },
                    )));
                    let token = format!("device:{id}");
                    *UNSAVED_ENTITY_SIGNAL.write() = Some(token.clone());
                    *DIRTY_EDITOR_SIGNAL.write() = Some(token);
                    selected.set(Some(id));
                }, Plus { size: 16, "aria-hidden": "true" } }
            }
            div { class: "search-field",
                Search { size: 15, "aria-hidden": "true" }
                input { class: "search", placeholder: "Search devices", value: "{query}", oninput: move |event| query.set(event.value()) }
            }
            button {
                class: "discovery-refresh",
                disabled: refresh_pending,
                aria_busy: refresh_pending,
                onclick: {
                    let runtime = runtime.clone();
                    let inventory = inventory.clone();
                    move |_| request_inventory_refresh(&runtime, &inventory, queued_revision, request_error)
                },
                span { "Connected interfaces" }
                small { "{refresh_summary}" }
            }
            small { class: status_class, role: "status", aria_live: "polite", "{status_text}" }
            if !inventory.rows.is_empty() {
                div { class: if discovery_open() { "discovery-list expanded" } else { "discovery-list" },
                    button {
                        class: "discovery-list__toggle",
                        aria_expanded: discovery_open(),
                        aria_controls: "connected-interface-list",
                        onclick: move |_| { let open = discovery_open(); discovery_open.set(!open); },
                        span { class: "disclosure-icon", aria_hidden: true,
                            if discovery_open() { ChevronDown { size: 13 } } else { ChevronRight { size: 13 } }
                        }
                        "Discovered interfaces"
                    }
                    if discovery_open() {
                        div { class: "discovery-list__items", id: "connected-interface-list",
                            for row in inventory.rows.iter().cloned() {
                                DiscoveryRow {
                                    key: "{row.selector.vendor_id}-{row.selector.product_id}-{row.selector.usage_page}-{row.selector.usage}",
                                    row,
                                    refresh_state: inventory.refresh_state.clone(),
                                    selected,
                                    query,
                                    discovery_open,
                                    navigation_locked,
                                    config: published.editable().clone(),
                                    revision: published.revision(),
                                    pending_draft,
                                }
                            }
                        }
                    }
                }
            }
            div { class: "entity-list__items",
                for (device, presence) in filtered_devices {
                    button {
                        key: "{device.id}",
                        class: if selected().as_deref() == Some(&device.id) { "entity-row selected" } else { "entity-row" },
                        disabled: navigation_locked && selected().as_deref() != Some(&device.id),
                        title: "{presence.title}",
                        aria_label: "{device.name}: {presence.label}",
                        onclick: {
                            let id = device.id.clone(); move |_| selected.set(Some(id.clone()))
                        },
                        span { class: presence.status_class, aria_hidden: true }
                        span { class: "entity-row__copy",
                            strong { "{device.name}" }
                            small {
                                span { class: "presence-label", "{presence.label}" }
                                " · VID {device.vid:04X} · PID {device.pid:04X}"
                            }
                        }
                    }
                }
            }
        }
        section { class: "workspace",
            if let Some(id) = selected() {
                if let Some(initial) = pending_draft().filter(|draft| draft.edited.id == id).or_else(|| {
                    devices.iter().find(|device| device.id == id).cloned().map(|device| DeviceDraft::edit(published.revision(), device))
                }) {
                    DeviceEditor { key: "{id}", initial, selected, pending_draft }
                } else {
                    EmptyState { title: "Select a device", copy: "Add a connected or manual HID interface, then reuse it across report actions." }
                }
            } else { EmptyState { title: "Select a device", copy: "Add a connected or manual HID interface, then reuse it across report actions." } }
        }
    }
}
