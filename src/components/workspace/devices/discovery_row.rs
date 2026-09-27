use dioxus::prelude::*;

use crate::{
    config::Device,
    hid::{HidInventoryRow, HidRefreshState, InterfaceSelector},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SelectorAliases {
    None,
    One(String),
    Multiple(usize),
}

pub(super) fn selector_aliases(devices: &[Device], selector: InterfaceSelector) -> SelectorAliases {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DiscoveryIntent {
    Adopt(InterfaceSelector),
    OpenSaved(InterfaceSelector),
}

fn format_selector(selector: InterfaceSelector) -> String {
    format!(
        "{:04X}:{:04X} · {:04X}:{:04X}",
        selector.vendor_id, selector.product_id, selector.usage_page, selector.usage
    )
}

#[derive(Props, Clone, PartialEq)]
pub(super) struct DiscoveryRowProps {
    pub row: HidInventoryRow,
    pub refresh_state: HidRefreshState,
    pub navigation_locked: bool,
    pub aliases: SelectorAliases,
    pub on_intent: EventHandler<DiscoveryIntent>,
}

#[component]
pub(super) fn DiscoveryRow(props: DiscoveryRowProps) -> Element {
    let row = props.row;
    let aliases = props.aliases;
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
    let selector_text = format_selector(row.selector);

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
                                onclick: move |_| props.on_intent.call(DiscoveryIntent::Adopt(row.selector)),
                                "Add"
                            }
                        },
                        SelectorAliases::One(_) => rsx! {
                            button {
                                class: "button secondary small",
                                disabled: props.navigation_locked,
                                onclick: move |_| props.on_intent.call(DiscoveryIntent::OpenSaved(row.selector)),
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
