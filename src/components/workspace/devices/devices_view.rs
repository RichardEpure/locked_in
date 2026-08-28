use dioxus::prelude::*;

use crate::{
    CAPTURE_TARGET_SIGNAL, CONFIG_SIGNAL, DIRTY_EDITOR_SIGNAL, UNSAVED_ENTITY_SIGNAL,
    config::Device, hid,
};

use super::{
    device_editor::DeviceEditor,
    discovery::{
        DISCOVERED_INTERFACES_SIGNAL, DISCOVERY_STATUS_SIGNAL, DiscoveryStatus,
        refresh_discovered_interfaces,
    },
};
use crate::components::workspace::{empty_state::EmptyState, selection::SelectionProps};

#[component]
pub(in crate::components::workspace) fn DevicesView(props: SelectionProps) -> Element {
    let mut selected = props.selected;
    let mut query = use_signal(String::new);
    let mut discovery_open = use_signal(|| false);
    use_effect(refresh_discovered_interfaces);
    let devices = CONFIG_SIGNAL.read().devices.clone();
    let discovered_count = DISCOVERED_INTERFACES_SIGNAL.read().len();
    let normalized_query = query().to_lowercase();
    let status = DISCOVERY_STATUS_SIGNAL();
    let is_refreshing = status == DiscoveryStatus::Refreshing;
    let (status_class, status_text) = match status {
        DiscoveryStatus::Idle => (
            "discovery-status",
            "Connected interfaces have not been refreshed".to_string(),
        ),
        DiscoveryStatus::Refreshing => (
            "discovery-status",
            "Refreshing connected interfaces...".to_string(),
        ),
        DiscoveryStatus::Ready(count) => (
            "discovery-status success",
            format!("Refresh complete: {count} found"),
        ),
        DiscoveryStatus::Failed(error) => {
            ("discovery-status error", format!("Refresh failed: {error}"))
        }
    };
    let navigation_locked =
        DIRTY_EDITOR_SIGNAL.read().is_some() || CAPTURE_TARGET_SIGNAL.read().is_some();
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
                    let mut config = CONFIG_SIGNAL.read().clone();
                    let id = config.next_id("device");
                    config.devices.push(Device { id: id.clone(), name: "New device".into(), report_length: 32, ..Device::default() });
                    *CONFIG_SIGNAL.write() = config;
                    let token = format!("device:{id}");
                    *UNSAVED_ENTITY_SIGNAL.write() = Some(token.clone());
                    *DIRTY_EDITOR_SIGNAL.write() = Some(token);
                    selected.set(Some(id));
                }, "+" }
            }
            input { class: "search", placeholder: "Search devices", value: "{query}", oninput: move |event| query.set(event.value()) }
            button {
                class: "discovery-refresh",
                disabled: is_refreshing,
                aria_busy: is_refreshing,
                onclick: move |_| refresh_discovered_interfaces(),
                span { "Connected interfaces" }
                small { if is_refreshing { "Refreshing..." } else { "{discovered_count} found · Refresh" } }
            }
            small { class: "{status_class}", role: "status", aria_live: "polite", "{status_text}" }
            if !DISCOVERED_INTERFACES_SIGNAL.read().is_empty() {
                div { class: if discovery_open() { "discovery-list expanded" } else { "discovery-list" },
                    button {
                        class: "discovery-list__toggle",
                        aria_expanded: discovery_open(),
                        aria_controls: "connected-interface-list",
                        onclick: move |_| { let open = discovery_open(); discovery_open.set(!open); },
                        span { class: "disclosure-icon", aria_hidden: true, if discovery_open() { "v" } else { ">" } }
                        "Adopt connected interface"
                    }
                    if discovery_open() {
                        div { class: "discovery-list__items", id: "connected-interface-list",
                            for interface in DISCOVERED_INTERFACES_SIGNAL() {
                                button { class: "discovery-row", disabled: navigation_locked, onclick: move |_| {
                                    let mut config = CONFIG_SIGNAL.read().clone();
                                    if let Some(existing) = config.devices.iter().find(|device|
                                        device.vid == interface.vendor_id && device.pid == interface.product_id
                                            && device.usage_page == interface.usage_page && device.usage == interface.usage
                                    ) {
                                        selected.set(Some(existing.id.clone()));
                                    } else {
                                        let id = config.next_id(&interface.name);
                                        config.devices.push(Device {
                                            id: id.clone(), name: interface.name.clone(), vid: interface.vendor_id,
                                            pid: interface.product_id, usage_page: interface.usage_page,
                                            usage: interface.usage, report_length: 32, report_id: 0,
                                        });
                                        *CONFIG_SIGNAL.write() = config;
                                        let token = format!("device:{id}");
                                        *UNSAVED_ENTITY_SIGNAL.write() = Some(token.clone());
                                        *DIRTY_EDITOR_SIGNAL.write() = Some(token);
                                        selected.set(Some(id));
                                    }
                                    query.set(String::new());
                                    discovery_open.set(false);
                                },
                                    strong { "{interface.name}" }
                                    small { "{interface.vendor_id:04X}:{interface.product_id:04X} · {interface.usage_page}:{interface.usage}" }
                                }
                            }
                        }
                    }
                }
            }
            div { class: "entity-list__items",
                for device in devices.iter().filter(|device| device.name.to_lowercase().contains(&normalized_query)).cloned() {
                    button { key: "{device.id}", class: if selected().as_deref() == Some(&device.id) { "entity-row selected" } else { "entity-row" }, disabled: navigation_locked && selected().as_deref() != Some(&device.id), onclick: {
                        let id = device.id.clone(); move |_| selected.set(Some(id.clone()))
                    }, span { class: if hid::is_connected(&device) { "status-dot online" } else { "status-dot" } }
                        span { class: "entity-row__copy", strong { "{device.name}" } small { "VID {device.vid:04X} · PID {device.pid:04X}" } }
                    }
                }
            }
        }
        section { class: "workspace",
            if let Some(id) = selected().filter(|id| devices.iter().any(|device| device.id == *id)) { DeviceEditor { key: "{id}", id, selected } }
            else { EmptyState { title: "Select a device", copy: "Add a connected or manual HID interface, then reuse it across report actions." } }
        }
    }
}
