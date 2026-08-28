use dioxus::prelude::*;

use crate::{
    CONFIG_SIGNAL, DIRTY_EDITOR_SIGNAL, UNSAVED_ENTITY_SIGNAL,
    automation_runtime::AutomationRuntime, config, hid::HidPresence,
};

use super::numeric_field::NumericField;
use crate::components::workspace::hid_inventory::{HidInventoryContext, hid_presence_view};

#[derive(Props, Clone, PartialEq)]
pub(super) struct EntityIdProps {
    id: String,
    selected: Signal<Option<String>>,
}

#[component]
pub(super) fn DeviceEditor(props: EntityIdProps) -> Element {
    let runtime = consume_context::<AutomationRuntime>();
    let inventory = consume_context::<HidInventoryContext>().current();
    let mut selected = props.selected;
    let delete_id = props.id.clone();
    let save_id = props.id.clone();
    let original = CONFIG_SIGNAL
        .read()
        .devices
        .iter()
        .find(|item| item.id == props.id)
        .cloned()
        .unwrap_or_default();
    let mut draft = use_signal(|| original.clone());
    let mut message = use_signal(|| None::<(bool, String)>);
    let mut delete_confirm = use_signal(|| false);
    let snapshot = draft();
    let editor_token = format!("device:{}", props.id);
    let is_new = UNSAVED_ENTITY_SIGNAL.read().as_deref() == Some(editor_token.as_str());
    let dirty = snapshot != original || is_new;
    let device_presence = inventory.presence(&snapshot);
    let presence = hid_presence_view(device_presence);
    let presence_copy = match device_presence {
        HidPresence::Connected => "One matching interface was found in the latest refresh",
        HidPresence::Disconnected => "Waiting for a matching interface",
        HidPresence::Ambiguous { .. } => "Dispatch is unavailable until only one match remains",
        HidPresence::Unknown => "Availability cannot be confirmed during refresh or after failure",
    };
    let cancel_original = original.clone();
    let cancel_id = props.id.clone();
    let cancel_token = editor_token.clone();
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
    use_drop(move || {
        if DIRTY_EDITOR_SIGNAL.read().as_deref() == Some(cleanup_token.as_str()) {
            *DIRTY_EDITOR_SIGNAL.write() = None;
        }
    });
    let references = CONFIG_SIGNAL
        .read()
        .automations
        .iter()
        .filter(|automation| {
            automation
                .cases
                .iter()
                .flat_map(|case| &case.actions)
                .chain(&automation.otherwise_actions)
                .any(|action| action.device_ids.contains(&props.id))
        })
        .map(|automation| automation.name.clone())
        .collect::<Vec<_>>();
    let references_text = references.join(", ");
    let delete_references = references.clone();
    let delete_references_text = references_text.clone();
    let delete_runtime = runtime.clone();
    let save_runtime = runtime.clone();
    rsx! {
        header { class: "workspace-header", div { div { class: "eyebrow", "HID DESTINATION" } h2 { "{snapshot.name}" }
                p { class: "device-presence", title: "{presence.title}",
                    span { class: presence.status_class, aria_hidden: true }
                    strong { "{presence.label}" }
                    " · {presence_copy}"
                }
            }
            button { class: "button danger-ghost", onclick: move |_| {
                if !delete_references.is_empty() {
                    message.set(Some((false, format!("Used by: {delete_references_text}"))));
                } else if delete_confirm() {
                    let mut next = CONFIG_SIGNAL.read().clone();
                    next.devices.retain(|device| device.id != delete_id);
                    match config::save(&next) {
                        Ok(()) => { delete_runtime.replace_config(next.clone()); *CONFIG_SIGNAL.write() = next; *DIRTY_EDITOR_SIGNAL.write() = None; *UNSAVED_ENTITY_SIGNAL.write() = None; selected.set(None); }
                        Err(error) => message.set(Some((false, format!("Delete failed: {error}")))),
                    }
                } else {
                    delete_confirm.set(true);
                    message.set(Some((false, "Click Delete again to confirm".into())));
                }
            }, "Delete" }
        }
        div { class: "editor-scroll",
            section { class: "editor-card", div { class: "section-heading", span { class: "step", "01" } div { h3 { "Identity" } p { "Name this device for use in report destinations" } } }
                div { class: "form-grid two", label { "Name" input { value: "{snapshot.name}", oninput: move |event| draft.write().name = event.value() } } label { "Stable ID" input { value: "{snapshot.id}", disabled: true } } }
            }
            section { class: "editor-card", div { class: "section-heading", span { class: "step", "02" } div { h3 { "HID interface" } p { "Enter decimal HID identifiers and report settings" } } }
                div { class: "form-grid three",
                    NumericField { label: "Vendor ID", value: snapshot.vid, on_change: move |value| draft.write().vid = value }
                    NumericField { label: "Product ID", value: snapshot.pid, on_change: move |value| draft.write().pid = value }
                    NumericField { label: "Usage page", value: snapshot.usage_page, on_change: move |value| draft.write().usage_page = value }
                    NumericField { label: "Usage", value: snapshot.usage, on_change: move |value| draft.write().usage = value }
                    NumericField { label: "Report length", value: snapshot.report_length, on_change: move |value| draft.write().report_length = value }
                    label { "Report ID" input { type: "number", min: "0", max: "255", value: "{snapshot.report_id}", oninput: move |event| if let Ok(value) = event.value().parse() { draft.write().report_id = value } } }
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
                if UNSAVED_ENTITY_SIGNAL.read().as_deref() == Some(cancel_token.as_str()) {
                    CONFIG_SIGNAL.write().devices.retain(|device| device.id != cancel_id);
                    *UNSAVED_ENTITY_SIGNAL.write() = None;
                    *DIRTY_EDITOR_SIGNAL.write() = None;
                    selected.set(None);
                } else {
                    draft.set(cancel_original.clone());
                }
            }, "Cancel" }
                button { class: "button primary", disabled: !dirty, onclick: move |_| {
                    let mut next = CONFIG_SIGNAL.read().clone();
                    if let Some(index) = next.devices.iter().position(|item| item.id == save_id) { next.devices[index] = draft(); }
                    let errors = next.validate();
                    if errors.is_empty() { match config::save(&next) { Ok(()) => { save_runtime.replace_config(next.clone()); *CONFIG_SIGNAL.write() = next; *UNSAVED_ENTITY_SIGNAL.write() = None; *DIRTY_EDITOR_SIGNAL.write() = None; message.set(Some((true, "Device saved".into()))); }, Err(error) => message.set(Some((false, error.to_string()))) } }
                    else { message.set(Some((false, errors[0].message.clone()))); }
                }, "Save device" }
            }
        }
    }
}
