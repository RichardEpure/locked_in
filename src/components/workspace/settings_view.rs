use std::process::Command;

use dioxus::{desktop::use_window, prelude::*};

use crate::{
    CONFIG_REVISION_SIGNAL, CONFIG_SIGNAL, DIRTY_EDITOR_SIGNAL, app_log,
    config::{self, LogLevel},
};

#[component]
pub(super) fn SettingsView() -> Element {
    let window = use_window();
    let reload_window = window.clone();
    let original = CONFIG_SIGNAL.read().settings.clone();
    let previous_startup = original.start_with_windows;
    let mut draft = use_signal(|| original.clone());
    let mut message = use_signal(String::new);
    let snapshot = draft();
    let cancel_original = original.clone();
    let effect_original = original.clone();
    use_effect(move || {
        if draft() != effect_original {
            *DIRTY_EDITOR_SIGNAL.write() = Some("settings".into());
        } else if DIRTY_EDITOR_SIGNAL.read().as_deref() == Some("settings") {
            *DIRTY_EDITOR_SIGNAL.write() = None;
        }
    });
    use_drop(move || {
        if DIRTY_EDITOR_SIGNAL.read().as_deref() == Some("settings") {
            *DIRTY_EDITOR_SIGNAL.write() = None;
        }
    });
    rsx! {
        section { class: "workspace settings-workspace",
            header { class: "workspace-header", div { div { class: "eyebrow", "APPLICATION" } h2 { "Settings" } p { "Startup, tray, configuration, and diagnostics" } } }
            div { class: "editor-scroll",
                section { class: "editor-card", div { class: "section-heading", span { class: "step", "01" } div { h3 { "Startup and tray" } p { "Locked In continues running when its window is closed" } } }
                    label { class: "setting-row", div { strong { "Start minimized" } small { "Hide the window after launch" } } input { type: "checkbox", checked: snapshot.start_minimized, onchange: move |event| draft.write().start_minimized = event.checked() } }
                    label { class: "setting-row", div { strong { "Close to tray" } small { "Keep automations active after closing the window" } } input { type: "checkbox", checked: snapshot.close_to_tray, onchange: move |event| draft.write().close_to_tray = event.checked() } }
                    label { class: "setting-row", div { strong { "Start with Windows" } small { "Launch Locked In when you sign in to Windows" } } input { type: "checkbox", checked: snapshot.start_with_windows, onchange: move |event| draft.write().start_with_windows = event.checked() } }
                }
                section { class: "editor-card", div { class: "section-heading", span { class: "step", "02" } div { h3 { "Configuration" } p { "Open the TOML file, then reload changes from disk" } } }
                    div { class: "settings-actions",
                        button { class: "button secondary", onclick: move |_| if let Ok(path) = config::config_path() { let _ = Command::new("notepad.exe").arg(path).spawn(); }, "Open config file" }
                        button { class: "button secondary", onclick: move |_| match config::Config::load() {
                            Ok(config) => match crate::win::set_start_with_windows(config.settings.start_with_windows) {
                                Ok(()) => {
                                    app_log::set_level(config.settings.log_level);
                                    reload_window.set_close_behavior(if config.settings.close_to_tray { dioxus::desktop::WindowCloseBehaviour::WindowHides } else { dioxus::desktop::WindowCloseBehaviour::WindowCloses });
                                    draft.set(config.settings.clone());
                                    *CONFIG_SIGNAL.write() = config;
                                    *CONFIG_REVISION_SIGNAL.write() += 1;
                                    *DIRTY_EDITOR_SIGNAL.write() = None;
                                    message.set("Configuration and runtime settings reloaded".into());
                                }
                                Err(error) => message.set(format!("Reload failed while applying startup setting: {error}")),
                            },
                            Err(error) => message.set(format!("Reload failed: {error}")),
                        }, "Reload from disk" }
                    }
                }
                section { class: "editor-card", div { class: "section-heading", span { class: "step", "03" } div { h3 { "Diagnostics" } p { "Choose log detail for automation and HID troubleshooting" } } }
                    div { class: "form-grid two", label { "Log level" select { value: log_level_name(snapshot.log_level), onchange: move |event| draft.write().log_level = parse_log_level(&event.value()), option { value: "error", "Error" } option { value: "info", "Info" } option { value: "debug", "Debug" } } }
                        div { class: "settings-actions align-end", button { class: "button secondary", onclick: move |_| if let Ok(path) = app_log::log_directory() { let _ = Command::new("explorer.exe").arg(path).spawn(); }, "Open log folder" } }
                    }
                }
            }
            footer { class: "save-bar", span { class: "message success", "{message}" }
                div { class: "toolbar",
                button { class: "button ghost", disabled: snapshot == original, onclick: move |_| { draft.set(cancel_original.clone()); *DIRTY_EDITOR_SIGNAL.write() = None; }, "Cancel" }
                button { class: "button primary", disabled: snapshot == original, onclick: move |_| {
                    let mut next = CONFIG_SIGNAL.read().clone(); next.settings = draft();
                    if let Err(error) = crate::win::set_start_with_windows(next.settings.start_with_windows) {
                        message.set(format!("Startup setting failed: {error}"));
                    } else {
                        match next.save() {
                            Ok(()) => {
                                app_log::set_level(next.settings.log_level);
                                window.set_close_behavior(if next.settings.close_to_tray { dioxus::desktop::WindowCloseBehaviour::WindowHides } else { dioxus::desktop::WindowCloseBehaviour::WindowCloses });
                                *CONFIG_SIGNAL.write() = next;
                                *DIRTY_EDITOR_SIGNAL.write() = None;
                                message.set("Settings saved".into());
                            }
                            Err(error) => {
                                match crate::win::set_start_with_windows(previous_startup) {
                                    Ok(()) => message.set(format!("Save failed: {error}")),
                                    Err(rollback_error) => message.set(format!("Save failed: {error}; startup rollback also failed: {rollback_error}")),
                                }
                            }
                        }
                    }
                }, "Save settings" }
                }
            }
        }
    }
}

fn log_level_name(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Error => "error",
        LogLevel::Info => "info",
        LogLevel::Debug => "debug",
    }
}

fn parse_log_level(value: &str) -> LogLevel {
    match value {
        "error" => LogLevel::Error,
        "debug" => LogLevel::Debug,
        _ => LogLevel::Info,
    }
}
