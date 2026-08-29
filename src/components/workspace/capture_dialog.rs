use dioxus::prelude::*;

use crate::{
    CAPTURE_GENERATION_SIGNAL, CAPTURED_WINDOW_SIGNAL, DIRTY_EDITOR_SIGNAL, cancel_capture,
};

use super::automations::{commit_captured_matcher, use_config_publication};

#[component]
pub(super) fn CaptureDialog() -> Element {
    let (coordinator, mut publication) = use_config_publication();
    let generation = *CAPTURE_GENERATION_SIGNAL.read();
    let captured = CAPTURED_WINDOW_SIGNAL
        .read()
        .as_ref()
        .filter(|captured| captured.belongs_to(generation, &None))
        .map(|captured| captured.window.clone())
        .unwrap_or_default();
    let mut automation_id = use_signal(String::new);
    let mut case_id = use_signal(String::new);
    let mut exception = use_signal(|| false);
    let mut message = use_signal(String::new);
    let published = publication.read().clone();
    let config = published.editable();
    let selected_cases = config
        .automations
        .iter()
        .find(|automation| automation.id == automation_id())
        .map(|automation| automation.cases.clone())
        .unwrap_or_default();
    let title = captured.title.clone().unwrap_or_default();
    let class = captured.class.clone().unwrap_or_default();
    let exe = captured
        .exe
        .as_ref()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_default();

    rsx! {
        div { class: "modal-backdrop",
            section { class: "capture-dialog",
                header { div { div { class: "eyebrow", "CAPTURED WINDOW" } h2 { "Assign matcher" } p { "Review the captured metadata, then choose an automation case." } }
                    button { class: "icon-button", aria_label: "Close", onclick: move |_| cancel_capture(), "×" }
                }
                div { class: "capture-metadata",
                    div { span { "Title" } code { "{title}" } }
                    div { span { "Class" } code { "{class}" } }
                    div { span { "Executable" } code { "{exe}" } }
                }
                div { class: "form-grid two",
                    label { "Automation" select { value: "{automation_id}", onchange: move |event| { automation_id.set(event.value()); case_id.set(String::new()); },
                        option { value: "", "Select automation" }
                        for automation in &config.automations { option { value: "{automation.id}", "{automation.name}" } }
                    } }
                    label { "Case" select { value: "{case_id}", disabled: automation_id().is_empty(), onchange: move |event| case_id.set(event.value()),
                        option { value: "", "Select case" }
                        for case in &selected_cases { option { value: "{case.id}", "{case.name}" } }
                    } }
                }
                label { class: "exception-toggle", input { type: "checkbox", checked: exception(), onchange: move |event| exception.set(event.checked()) } "Add as an exception matcher" }
                if !message().is_empty() { p { class: "message error", "{message}" } }
                footer { class: "toolbar modal-actions",
                    button { class: "button ghost", onclick: move |_| cancel_capture(), "Cancel" }
                    button { class: "button primary", disabled: automation_id().is_empty() || case_id().is_empty(), onclick: move |_| {
                        let target_automation_id = automation_id();
                        if DIRTY_EDITOR_SIGNAL.read().is_some() {
                            message.set("Save or cancel the open draft, or use Capture next inside that editor".into());
                            return;
                        }
                        let expected_revision = publication.read().revision();
                        match commit_captured_matcher(&coordinator, expected_revision, &target_automation_id, &case_id(), exception(), &captured) {
                            Ok(published) => {
                                publication.set(published);
                                cancel_capture();
                            }
                            Err(error) => message.set(format!("Could not save matcher; your capture and selections are preserved: {error}")),
                        }
                    }, "Add matcher" }
                }
            }
        }
    }
}
