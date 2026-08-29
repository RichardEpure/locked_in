use std::sync::Arc;

use dioxus::prelude::*;

use crate::{
    DIRTY_EDITOR_SIGNAL, UNSAVED_ENTITY_SIGNAL, config::ConfigCoordinator, hid::HidPresence,
};

use super::{
    draft::{DeviceDraft, device_references},
    numeric_field::NumericField,
};
use crate::components::workspace::{
    hid_inventory::{HidInventoryContext, hid_presence_view},
    published_config::PublishedConfigContext,
};

#[derive(Props, Clone, PartialEq)]
pub(super) struct DeviceEditorProps {
    initial: DeviceDraft,
    selected: Signal<Option<String>>,
    pending_draft: Signal<Option<DeviceDraft>>,
}

#[component]
pub(super) fn DeviceEditor(props: DeviceEditorProps) -> Element {
    let coordinator = consume_context::<Option<Arc<ConfigCoordinator>>>()
        .expect("configuration coordinator must be available after a successful load");
    let publication_context = consume_context::<PublishedConfigContext>();
    let published = publication_context.current();
    let inventory = consume_context::<HidInventoryContext>().current();
    let mut selected = props.selected;
    let mut pending_draft = props.pending_draft;
    let mut editor = use_signal(|| props.initial.clone());
    let mut message = use_signal(|| None::<(bool, String)>);
    let mut delete_confirm = use_signal(|| false);
    let snapshot = editor();
    let device = snapshot.edited.clone();
    let editor_token = format!("device:{}", device.id);
    let is_new = snapshot.is_new();
    let dirty = snapshot.is_dirty();
    let device_presence = inventory.presence(&device);
    let presence = hid_presence_view(device_presence);
    let presence_copy = match device_presence {
        HidPresence::Connected => "One matching interface was found in the latest refresh",
        HidPresence::Disconnected => "Waiting for a matching interface",
        HidPresence::Ambiguous { .. } => "Dispatch is unavailable until only one match remains",
        HidPresence::Unknown => "Availability cannot be confirmed during refresh or after failure",
    };

    use_effect(move || {
        let publication = publication_context.current();
        let mut synchronized = editor();
        if synchronized.refresh_if_clean(&publication) {
            editor.set(synchronized);
        }
    });

    let effect_token = editor_token.clone();
    use_effect(move || {
        if editor().is_dirty() {
            *DIRTY_EDITOR_SIGNAL.write() = Some(effect_token.clone());
        } else if DIRTY_EDITOR_SIGNAL.read().as_deref() == Some(effect_token.as_str()) {
            *DIRTY_EDITOR_SIGNAL.write() = None;
        }
    });
    let cleanup_token = editor_token.clone();
    use_drop(move || {
        if DIRTY_EDITOR_SIGNAL.read().as_deref() == Some(cleanup_token.as_str()) {
            *DIRTY_EDITOR_SIGNAL.write() = None;
        }
    });

    let references = device_references(published.editable(), &device.id);
    let references_text = references.join(", ");
    let delete_references = references.clone();
    let coordinator_for_delete = coordinator.clone();
    let coordinator_for_save = coordinator.clone();
    rsx! {
        header { class: "workspace-header", div { div { class: "eyebrow", "HID DESTINATION" } h2 { "{device.name}" }
                p { class: "device-presence", title: "{presence.title}",
                    span { class: presence.status_class, aria_hidden: true }
                    strong { "{presence.label}" }
                    " · {presence_copy}"
                }
            }
            button { class: "button danger-ghost", onclick: move |_| {
                if !delete_references.is_empty() {
                    message.set(Some((false, format!("Used by: {}", delete_references.join(", ")))));
                } else if delete_confirm() {
                    let state = editor();
                    match state.delete(&coordinator_for_delete, &delete_references) {
                        Ok(_) => {
                            pending_draft.set(None);
                            *DIRTY_EDITOR_SIGNAL.write() = None;
                            *UNSAVED_ENTITY_SIGNAL.write() = None;
                            selected.set(None);
                        }
                        Err(error) => message.set(Some((false, error.to_string()))),
                    }
                } else {
                    delete_confirm.set(true);
                    message.set(Some((false, "Click Delete again to confirm".into())));
                }
            }, "Delete" }
        }
        div { class: "editor-scroll",
            section { class: "editor-card", div { class: "section-heading", span { class: "step", "01" } div { h3 { "Identity" } p { "Name this device for use in report destinations" } } }
                div { class: "form-grid two", label { "Name" input { value: "{device.name}", oninput: move |event| editor.write().edited.name = event.value() } } label { "Stable ID" input { value: "{device.id}", disabled: true } } }
            }
            section { class: "editor-card", div { class: "section-heading", span { class: "step", "02" } div { h3 { "HID interface" } p { "Enter decimal HID identifiers and report settings" } } }
                div { class: "form-grid three",
                    NumericField { label: "Vendor ID", value: device.vid, on_change: move |value| editor.write().edited.vid = value }
                    NumericField { label: "Product ID", value: device.pid, on_change: move |value| editor.write().edited.pid = value }
                    NumericField { label: "Usage page", value: device.usage_page, on_change: move |value| editor.write().edited.usage_page = value }
                    NumericField { label: "Usage", value: device.usage, on_change: move |value| editor.write().edited.usage = value }
                    NumericField { label: "Report length", value: device.report_length, on_change: move |value| editor.write().edited.report_length = value }
                    label { "Report ID" input { type: "number", min: "0", max: "255", value: "{device.report_id}", oninput: move |event| if let Ok(value) = event.value().parse() { editor.write().edited.report_id = value } } }
                }
                div { class: "device-note", strong { "Use a connected device" } p { "Use Connected interfaces to add or select a detected device, or enter HID values manually." } }
            }
            if !references.is_empty() { section { class: "editor-card references", h3 { "Used by" } p { "{references_text}" } } }
        }
        footer { class: "save-bar",
            div { class: "save-bar__status",
                if is_new { span { class: "message warning", role: "status", "Navigation is locked while this device is unsaved. Save or Cancel to continue." } }
                if let Some((success, text)) = message() { span { class: if success { "message success" } else { "message error" }, "{text}" } }
            }
            div { class: "toolbar", button { class: "button ghost", disabled: !dirty, onclick: move |_| {
                let mut state = editor();
                if state.cancel(&published) {
                    editor.set(state);
                } else {
                    pending_draft.set(None);
                    selected.set(None);
                }
                *UNSAVED_ENTITY_SIGNAL.write() = None;
                *DIRTY_EDITOR_SIGNAL.write() = None;
                message.set(None);
            }, "Cancel" }
                button { class: "button primary", disabled: !dirty, onclick: move |_| {
                    let mut state = editor();
                    let was_new = state.is_new();
                    match state.save(&coordinator_for_save) {
                        Ok(_) => {
                            editor.set(state.clone());
                            if was_new {
                                pending_draft.set(Some(state));
                            }
                            *UNSAVED_ENTITY_SIGNAL.write() = None;
                            *DIRTY_EDITOR_SIGNAL.write() = None;
                            message.set(Some((true, "Device saved".into())));
                        }
                        Err(error) => message.set(Some((false, format!("Save failed: {error}")))),
                    }
                }, "Save device" }
            }
        }
    }
}
