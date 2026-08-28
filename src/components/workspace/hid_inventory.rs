use std::sync::Arc;

use dioxus::prelude::*;

use crate::hid::{HidInventory, HidPresence};

#[derive(Clone, Copy)]
pub(super) struct HidInventoryContext(ReadSignal<Arc<HidInventory>>);

impl HidInventoryContext {
    pub(super) fn new(inventory: Signal<Arc<HidInventory>>) -> Self {
        Self(ReadSignal::new(inventory))
    }

    pub(super) fn current(self) -> Arc<HidInventory> {
        self.0()
    }
}

pub(super) struct HidPresenceView {
    pub status_class: &'static str,
    pub label: String,
    pub title: String,
}

pub(super) fn hid_presence_view(presence: HidPresence) -> HidPresenceView {
    match presence {
        HidPresence::Connected => HidPresenceView {
            status_class: "status-dot online",
            label: "Connected".into(),
            title: "Exactly one matching HID interface was found in the latest refresh".into(),
        },
        HidPresence::Disconnected => HidPresenceView {
            status_class: "status-dot disconnected",
            label: "Disconnected".into(),
            title: "No matching HID interface was found in the latest refresh".into(),
        },
        HidPresence::Ambiguous { matches } => HidPresenceView {
            status_class: "status-dot warning",
            label: format!("Ambiguous ({matches})"),
            title: format!("{matches} matching HID interfaces were found; dispatch is unavailable"),
        },
        HidPresence::Unknown => HidPresenceView {
            status_class: "status-dot unknown",
            label: "Unknown".into(),
            title: "HID availability is unknown until a refresh completes".into(),
        },
    }
}
