use dioxus::prelude::*;

use crate::{
    CONFIG_SIGNAL,
    config::{Automation, SendAction},
    hid,
};

use super::{
    INVALID_REPORT_IDS,
    mutations::{remove_action, with_action_mut},
};

#[derive(Props, Clone, PartialEq)]
pub(super) struct ActionEditorProps {
    draft: Signal<Automation>,
    case_index: Option<usize>,
    action_index: usize,
    action: SendAction,
}

#[component]
pub(super) fn ActionEditor(props: ActionEditorProps) -> Element {
    let mut draft = props.draft;
    let action = props.action;
    let devices = CONFIG_SIGNAL.read().devices.clone();
    let mut test_result = use_signal(|| None::<(bool, String)>);
    let mut report_input = use_signal(|| hex::encode(&action.report));
    let report_text = report_input();
    let parsed_report = parse_report_hex(&report_text);
    let report_length = parsed_report.as_ref().map_or(action.report.len(), Vec::len);
    let action_id = action.id.clone();
    let effect_action_id = action_id.clone();
    use_effect(move || {
        if parse_report_hex(&report_input()).is_ok() {
            INVALID_REPORT_IDS.write().remove(&effect_action_id);
        } else {
            INVALID_REPORT_IDS.write().insert(effect_action_id.clone());
        }
    });
    use_drop(move || {
        INVALID_REPORT_IDS.write().remove(&action_id);
    });
    rsx! {
        div { class: "action-card",
            div { class: "action-card__top",
                input { class: "action-label", placeholder: "Optional action label", value: "{action.label}", oninput: move |event| with_action_mut(&mut draft, props.case_index, props.action_index, |action| action.label = event.value()) }
                button { class: "button secondary small", onclick: {
                    let mut action = action.clone();
                    move |_| {
                        let Ok(report) = parse_report_hex(&report_input()) else {
                            test_result.set(Some((false, "Report must contain complete hexadecimal bytes".into())));
                            return;
                        };
                        action.report = report;
                        let config = CONFIG_SIGNAL.read();
                        let validation_errors = config.validate_action(&action);
                        if !validation_errors.is_empty() {
                            test_result.set(Some((false, validation_errors.iter().map(|error| error.message.as_str()).collect::<Vec<_>>().join("; "))));
                            return;
                        }
                        let mut failures = Vec::new();
                        let mut sent = 0;
                        for device_id in &action.device_ids {
                            if let Some(device) = config.devices.iter().find(|device| device.id == *device_id) {
                                match device.send_report(&action.report) { Ok(_) => sent += 1, Err(error) => failures.push(format!("{}: {error}", device.name)) }
                            }
                        }
                        if failures.is_empty() && sent > 0 { test_result.set(Some((true, format!("Sent to {sent} device(s)")))); }
                        else { test_result.set(Some((false, if failures.is_empty() { "Select a destination".into() } else { failures.join("; ") }))); }
                    }
                }, "Test" }
                button { class: "icon-button danger", title: "Remove action", onclick: move |_| remove_action(&mut draft, props.case_index, props.action_index), "×" }
            }
            div { class: "action-fields",
                label { "Report (hex)" div { class: if parsed_report.is_ok() { "hex-input" } else { "hex-input invalid" }, span { "0x" } input { value: "{report_text}", placeholder: "87", oninput: move |event| {
                    let value = event.value();
                    report_input.set(value.clone());
                    if let Ok(bytes) = parse_report_hex(&value) { with_action_mut(&mut draft, props.case_index, props.action_index, |action| action.report = bytes); }
                } } small { if parsed_report.is_ok() { "{report_length} bytes" } else { "invalid" } } } }
                div { class: "destinations", span { class: "field-label", "Destinations" }
                    if devices.is_empty() { small { class: "muted-copy", "Add a device first" } }
                    for device in devices {
                        label { class: "destination-chip",
                            input { type: "checkbox", checked: action.device_ids.contains(&device.id), onchange: {
                                let id = device.id.clone();
                                move |event| {
                                    with_action_mut(&mut draft, props.case_index, props.action_index, |action| {
                                        if event.checked() && !action.device_ids.contains(&id) { action.device_ids.push(id.clone()); }
                                        else if !event.checked() { action.device_ids.retain(|item| item != &id); }
                                    });
                                }
                            } }
                            span { class: if hid::is_connected(&device) { "status-dot online" } else { "status-dot" } }
                            "{device.name}"
                            small { "{device.report_length} B" }
                        }
                    }
                }
            }
            if let Some((success, text)) = test_result() { small { class: if success { "message success" } else { "message error" }, "{text}" } }
        }
    }
}

fn parse_report_hex(value: &str) -> Result<Vec<u8>, hex::FromHexError> {
    hex::decode(
        value
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>(),
    )
}
