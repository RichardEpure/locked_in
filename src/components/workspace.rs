use std::process::Command;

use dioxus::prelude::*;

use crate::{
    CAPTURE_TARGET_SIGNAL, CAPTURED_WINDOW_SIGNAL, CONFIG_LOAD_ERROR, DIRTY_EDITOR_SIGNAL,
    automation_runtime::{AutomationRuntime, RuntimePhase},
    config,
};

use self::{
    automations::AutomationsView, capture_dialog::CaptureDialog, devices::DevicesView,
    settings_view::SettingsView,
};

mod automations;
mod capture_dialog;
mod devices;
mod empty_state;
mod hid_inventory;
mod published_config;
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
                    p { "Locked In is running without active configuration and will not overwrite an existing configuration file. Correct the startup error, then restart the application." }
                    pre { "{error}" }
                    button { class: "button secondary", onclick: move |_| if let Ok(path) = config::config_path() { let _ = Command::new("notepad.exe").arg(path).spawn(); }, "Open config file" }
                }
            }
        };
    }
    let mut section = use_signal(|| Section::Automations);
    let selected_automation = use_signal(|| None::<String>);
    let selected_device = use_signal(|| None::<String>);
    let publication_receiver = consume_context::<
        Option<tokio::sync::watch::Receiver<std::sync::Arc<config::PublishedConfig>>>,
    >()
    .expect("configuration publication context must be available after a successful load");
    let initial_publication = publication_receiver.borrow().clone();
    let mut publication = use_signal(move || initial_publication);
    use_context_provider(move || published_config::PublishedConfigContext::new(publication));
    let runtime = consume_context::<AutomationRuntime>();
    let status_receiver = runtime.subscribe_status();
    let initial_status = status_receiver.borrow().clone();
    let mut runtime_status = use_signal(move || initial_status);
    let inventory_receiver = runtime.subscribe_hid_inventory();
    let initial_inventory = inventory_receiver.borrow().clone();
    let mut hid_inventory = use_signal(move || initial_inventory);
    use_context_provider(move || hid_inventory::HidInventoryContext::new(hid_inventory));
    use_future(move || {
        let mut receiver = publication_receiver.clone();
        async move {
            while receiver.changed().await.is_ok() {
                publication.set(receiver.borrow_and_update().clone());
            }
        }
    });
    use_future(move || {
        let mut receiver = status_receiver.clone();
        async move {
            while receiver.changed().await.is_ok() {
                runtime_status.set(receiver.borrow_and_update().clone());
            }
        }
    });
    use_future(move || {
        let mut receiver = inventory_receiver.clone();
        async move {
            while receiver.changed().await.is_ok() {
                hid_inventory.set(receiver.borrow_and_update().clone());
            }
        }
    });
    let status = runtime_status();
    let (status_class, status_text) = match status.phase {
        RuntimePhase::Starting => ("status-dot", "Automation starting"),
        RuntimePhase::Active => ("status-dot online", "Automation active"),
        RuntimePhase::Degraded => ("status-dot warning", "Automation active with warnings"),
        RuntimePhase::Unavailable => ("status-dot error", "Automation unavailable"),
        RuntimePhase::Stopping => ("status-dot", "Automation stopping"),
        RuntimePhase::Stopped => ("status-dot error", "Automation stopped"),
    };
    let status_detail = status.detail.unwrap_or_default();
    let status_tooltip = if status_detail.is_empty() {
        status_text.to_string()
    } else {
        format!("{status_text}: {status_detail}")
    };
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
                div {
                    class: "app-nav__status",
                    role: "status",
                    aria_label: status_text,
                    title: "{status_tooltip}",
                    span { class: status_class }
                    span { class: "app-nav__status-label", "{status_text}" }
                }
            }
            match section() {
                Section::Automations => rsx! { AutomationsView { selected: selected_automation } },
                Section::Devices => rsx! { DevicesView { selected: selected_device } },
                Section::Settings => rsx! { SettingsView {} },
            }
            if CAPTURED_WINDOW_SIGNAL.read().as_ref().is_some_and(|captured|
                captured.belongs_to(*crate::CAPTURE_GENERATION_SIGNAL.read(), &None)
            ) && CAPTURE_TARGET_SIGNAL.read().is_none() {
                CaptureDialog {}
            }
        }
    }
}
