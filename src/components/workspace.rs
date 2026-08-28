use std::process::Command;

use dioxus::prelude::*;

use crate::{
    CAPTURE_TARGET_SIGNAL, CAPTURED_WINDOW_SIGNAL, CONFIG_LOAD_ERROR, DIRTY_EDITOR_SIGNAL,
    HID_CACHE_REVISION_SIGNAL, SERVICE_READY, config,
};

use self::{
    automations::AutomationsView, capture_dialog::CaptureDialog, devices::DevicesView,
    settings_view::SettingsView,
};

mod automations;
mod capture_dialog;
mod devices;
mod empty_state;
mod selection;
mod settings_view;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Automations,
    Devices,
    Settings,
}

#[component]
pub(super) fn Workspace() -> Element {
    if let Some(error) = CONFIG_LOAD_ERROR.as_ref() {
        return rsx! {
            div { class: "load-error",
                div { class: "brand", span { class: "brand__mark", "LI" } span { "Locked In" } }
                section { h1 { "Configuration could not be loaded" }
                    p { "Locked In will not overwrite this file. Convert it to schema version 2, then restart the application." }
                    pre { "{error}" }
                    button { class: "button secondary", onclick: move |_| if let Ok(path) = config::config_path() { let _ = Command::new("notepad.exe").arg(path).spawn(); }, "Open config file" }
                }
            }
        };
    }
    let mut section = use_signal(|| Section::Automations);
    let selected_automation = use_signal(|| None::<String>);
    let selected_device = use_signal(|| None::<String>);
    let service_ready = SERVICE_READY.load(std::sync::atomic::Ordering::Relaxed);
    let _hid_cache_revision = *HID_CACHE_REVISION_SIGNAL.read();
    let navigation_locked =
        DIRTY_EDITOR_SIGNAL.read().is_some() || CAPTURE_TARGET_SIGNAL.read().is_some();

    rsx! {
        div {
            class: "app-shell",
            tabindex: "0",
            autofocus: true,
            onkeydown: move |event| {
                let key = event.key().to_string().to_lowercase();
                if event.modifiers().contains(Modifiers::CONTROL) {
                    let script = match key.as_str() {
                        "n" => Some("document.querySelector('[aria-label^=\"New\"].icon-button.primary')?.click()"),
                        "s" => Some("document.querySelector('.save-bar .button.primary:not([disabled])')?.click()"),
                        "f" => Some("document.querySelector('.search')?.focus()"),
                        _ => None,
                    };
                    if let Some(script) = script {
                        event.prevent_default();
                        spawn(async move { let _ = document::eval(script).await; });
                    }
                } else if key == "escape" {
                    spawn(async move { let _ = document::eval("const button = document.querySelector('.modal-backdrop [aria-label=\"Close\"]') || document.querySelector('.save-bar .button.ghost:not([disabled])'); button?.click()").await; });
                } else if key == "delete" {
                    spawn(async move { let _ = document::eval("if (!['INPUT','SELECT','TEXTAREA'].includes(document.activeElement?.tagName)) document.querySelector('.workspace-header .danger-ghost')?.click()").await; });
                }
            },
            nav {
                class: "app-nav",
                div { class: "brand", span { class: "brand__mark", "LI" } span { "Locked In" } }
                button {
                    class: if section() == Section::Automations { "nav-item active" } else { "nav-item" },
                    disabled: navigation_locked && section() != Section::Automations,
                    onclick: move |_| section.set(Section::Automations),
                    span { class: "nav-item__icon", "A" }
                    span { class: "nav-item__label", "Automations" }
                }
                button {
                    class: if section() == Section::Devices { "nav-item active" } else { "nav-item" },
                    disabled: navigation_locked && section() != Section::Devices,
                    onclick: move |_| section.set(Section::Devices),
                    span { class: "nav-item__icon", "D" }
                    span { class: "nav-item__label", "Devices" }
                }
                button {
                    class: if section() == Section::Settings { "nav-item active" } else { "nav-item" },
                    disabled: navigation_locked && section() != Section::Settings,
                    onclick: move |_| section.set(Section::Settings),
                    span { class: "nav-item__icon", "S" }
                    span { class: "nav-item__label", "Settings" }
                }
                div { class: "app-nav__status", span { class: if service_ready { "status-dot online" } else { "status-dot error" } } if service_ready { "Automation service active" } else { "Automation service unavailable" } }
            }
            match section() {
                Section::Automations => rsx! { AutomationsView { selected: selected_automation } },
                Section::Devices => rsx! { DevicesView { selected: selected_device } },
                Section::Settings => rsx! { SettingsView {} },
            }
            if CAPTURED_WINDOW_SIGNAL.read().is_some() && CAPTURE_TARGET_SIGNAL.read().is_none() {
                CaptureDialog {}
            }
        }
    }
}
