use dioxus::prelude::*;

use crate::{
    CONFIG_SIGNAL, DIRTY_EDITOR_SIGNAL, UNSAVED_ENTITY_SIGNAL,
    config::Device,
    hid::{HidInventoryRow, HidRefreshState, InterfaceSelector},
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum SelectorAliases {
    None,
    One(String),
    Multiple(usize),
}

fn selector_aliases(devices: &[Device], selector: InterfaceSelector) -> SelectorAliases {
    let matches = devices
        .iter()
        .filter(|device| InterfaceSelector::from(*device) == selector)
        .map(|device| device.id.clone())
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => SelectorAliases::None,
        [id] => SelectorAliases::One(id.clone()),
        aliases => SelectorAliases::Multiple(aliases.len()),
    }
}

#[derive(Props, Clone, PartialEq)]
pub(super) struct DiscoveryRowProps {
    pub row: HidInventoryRow,
    pub refresh_state: HidRefreshState,
    pub selected: Signal<Option<String>>,
    pub query: Signal<String>,
    pub discovery_open: Signal<bool>,
    pub navigation_locked: bool,
}

#[component]
pub(super) fn DiscoveryRow(props: DiscoveryRowProps) -> Element {
    let row = props.row;
    let aliases = selector_aliases(&CONFIG_SIGNAL.read().devices, row.selector);
    let available = props.refresh_state == HidRefreshState::Ready && row.match_count == 1;
    let (row_class, state_class, state_text, state_title) = match &props.refresh_state {
        HidRefreshState::Ready if row.match_count == 1 => (
            "discovery-row",
            "discovery-row__state success",
            "Available".to_string(),
            "Exactly one connected interface matches this selector".to_string(),
        ),
        HidRefreshState::Ready => (
            "discovery-row unavailable",
            "discovery-row__state warning",
            format!("Ambiguous: {} matches", row.match_count),
            format!(
                "{} connected interfaces share this selector; adoption and dispatch are unavailable",
                row.match_count
            ),
        ),
        HidRefreshState::Refreshing => (
            "discovery-row unavailable stale",
            "discovery-row__state",
            "Stale while refreshing".to_string(),
            "This retained row is unavailable until refresh completes".to_string(),
        ),
        HidRefreshState::Failed { .. } => (
            "discovery-row unavailable stale",
            "discovery-row__state error",
            "Stale and unavailable".to_string(),
            "The last refresh failed; this retained row cannot be adopted".to_string(),
        ),
        HidRefreshState::NotAttempted => (
            "discovery-row unavailable stale",
            "discovery-row__state",
            "Unavailable".to_string(),
            "Refresh connected interfaces before adopting this row".to_string(),
        ),
    };
    let selector_text = format!(
        "{:04X}:{:04X} · {}:{}",
        row.selector.vendor_id,
        row.selector.product_id,
        row.selector.usage_page,
        row.selector.usage
    );

    rsx! {
        div {
            class: row_class,
            strong { "{row.name}" }
            small { "{selector_text}" }
            div { class: "discovery-row__footer",
                span { class: state_class, title: "{state_title}", "{state_text}" }
                if available {
                    match aliases {
                        SelectorAliases::None => rsx! {
                            button {
                                class: "button secondary small",
                                disabled: props.navigation_locked,
                                onclick: {
                                    let row = row.clone();
                                    let mut selected = props.selected;
                                    let mut query = props.query;
                                    let mut discovery_open = props.discovery_open;
                                    move |_| {
                                        let mut config = CONFIG_SIGNAL.read().clone();
                                        let id = config.next_id(&row.name);
                                        config.devices.push(Device {
                                            id: id.clone(),
                                            name: row.name.clone(),
                                            vid: row.selector.vendor_id,
                                            pid: row.selector.product_id,
                                            usage_page: row.selector.usage_page,
                                            usage: row.selector.usage,
                                            report_length: 32,
                                            report_id: 0,
                                        });
                                        *CONFIG_SIGNAL.write() = config;
                                        let token = format!("device:{id}");
                                        *UNSAVED_ENTITY_SIGNAL.write() = Some(token.clone());
                                        *DIRTY_EDITOR_SIGNAL.write() = Some(token);
                                        selected.set(Some(id));
                                        query.set(String::new());
                                        discovery_open.set(false);
                                    }
                                },
                                "Add"
                            }
                        },
                        SelectorAliases::One(id) => rsx! {
                            button {
                                class: "button secondary small",
                                disabled: props.navigation_locked,
                                onclick: {
                                    let mut selected = props.selected;
                                    let mut query = props.query;
                                    let mut discovery_open = props.discovery_open;
                                    move |_| {
                                        selected.set(Some(id.clone()));
                                        query.set(String::new());
                                        discovery_open.set(false);
                                    }
                                },
                                "Open saved"
                            }
                        },
                        SelectorAliases::Multiple(count) => rsx! {
                            span { class: "discovery-row__aliases", "Configured {count} times" }
                        },
                    }
                } else {
                    span { class: "discovery-row__unavailable", "Not adoptable" }
                    if let SelectorAliases::Multiple(count) = aliases {
                        span { class: "discovery-row__aliases", "Configured {count} times" }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
