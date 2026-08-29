use std::{process::Command, sync::Arc};

use dioxus::prelude::*;

use crate::{
    DIRTY_EDITOR_SIGNAL,
    components::PublishedConfigContext,
    config::{
        self, ConfigCoordinator, ConfigCoordinatorError, ConfigWarning, LogLevel, PublishedConfig,
        Settings,
    },
};

#[component]
pub(super) fn SettingsView() -> Element {
    let coordinator = consume_context::<Option<Arc<ConfigCoordinator>>>()
        .expect("settings require an available configuration coordinator");
    let paths = consume_context::<config::ApplicationPaths>();
    let publication_context = consume_context::<PublishedConfigContext>();
    let published = publication_context
        .current()
        .expect("settings require a published configuration");
    let initial = published.clone();
    let mut base = use_signal(|| initial.clone());
    let mut draft = use_signal(|| initial.editable().settings.clone());
    let mut message = use_signal(|| None::<(&'static str, String)>);

    use_effect(move || {
        let Some(observed) = publication_context.current() else {
            return;
        };
        let current_base = base.read();
        let is_clean = *draft.read() == current_base.editable().settings;
        if observed.revision() > current_base.revision() && is_clean {
            let settings = observed.editable().settings.clone();
            drop(current_base);
            base.set(observed);
            draft.set(settings);
        }
    });

    let snapshot = draft();
    let original = base.read().editable().settings.clone();
    let expected_revision = base.read().revision();
    let cancel_original = original.clone();
    let publication_warning = config_warning_message(published.warnings());
    use_effect(move || {
        if draft() != base.read().editable().settings {
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
                        button { class: "button secondary", onclick: {
                            let config_path = paths.config_path();
                            move |_| { let _ = Command::new("notepad.exe").arg(&config_path).spawn(); }
                        }, "Open config file" }
                        button { class: "button secondary", onclick: {
                            let coordinator = coordinator.clone();
                            move |_| match reload_settings(&coordinator) {
                                Ok(reloaded) => {
                                    let warning = config_warning_message(reloaded.warnings());
                                    let settings = reloaded.editable().settings.clone();
                                    base.set(reloaded);
                                    draft.set(settings);
                                    *DIRTY_EDITOR_SIGNAL.write() = None;
                                    message.set(Some(match warning {
                                        Some(warning) => ("message warning", warning),
                                        None => ("message success", "Configuration reloaded".into()),
                                    }));
                                }
                                Err(error) => {
                                    message.set(Some(("message error", config_error_message(&error))));
                                }
                            }
                        }, "Reload from disk" }
                    }
                }
                section { class: "editor-card", div { class: "section-heading", span { class: "step", "03" } div { h3 { "Diagnostics" } p { "Choose log detail for automation and HID troubleshooting" } } }
                    div { class: "form-grid two", label { "Log level" select { value: log_level_name(snapshot.log_level), onchange: move |event| draft.write().log_level = parse_log_level(&event.value()), option { value: "error", "Error" } option { value: "info", "Info" } option { value: "debug", "Debug" } } }
                        div { class: "settings-actions align-end", button { class: "button secondary", onclick: {
                            let log_directory = paths.log_directory();
                            move |_| { let _ = Command::new("explorer.exe").arg(&log_directory).spawn(); }
                        }, "Open log folder" } }
                    }
                }
            }
            footer { class: "save-bar",
                if let Some((class, text)) = message() {
                    span { class, role: "status", aria_live: "polite", "{text}" }
                } else if let Some(warning) = publication_warning {
                    span { class: "message warning", role: "status", "{warning}" }
                }
                div { class: "toolbar",
                    button { class: "button ghost", disabled: snapshot == original, onclick: move |_| { draft.set(cancel_original.clone()); *DIRTY_EDITOR_SIGNAL.write() = None; message.set(None); }, "Cancel" }
                    button { class: "button primary", disabled: snapshot == original, onclick: {
                        let coordinator = coordinator.clone();
                        move |_| match save_settings(&coordinator, expected_revision, draft()) {
                            Ok(saved) => {
                                let warning = config_warning_message(saved.warnings());
                                let settings = saved.editable().settings.clone();
                                base.set(saved);
                                draft.set(settings);
                                *DIRTY_EDITOR_SIGNAL.write() = None;
                                message.set(Some(match warning {
                                    Some(warning) => ("message warning", warning),
                                    None => ("message success", "Settings saved".into()),
                                }));
                            }
                            Err(error) => {
                                if matches!(error, ConfigCoordinatorError::StaleRevision { .. }) {
                                    base.set(coordinator.current());
                                }
                                message.set(Some(("message error", config_error_message(&error))));
                            }
                        }
                    }, "Save settings" }
                }
            }
        }
    }
}

fn save_settings(
    coordinator: &ConfigCoordinator,
    expected_revision: u64,
    settings: Settings,
) -> Result<Arc<PublishedConfig>, ConfigCoordinatorError> {
    coordinator.update(expected_revision, move |current| {
        let mut candidate = current.clone();
        candidate.settings = settings;
        candidate
    })
}

fn reload_settings(
    coordinator: &ConfigCoordinator,
) -> Result<Arc<PublishedConfig>, ConfigCoordinatorError> {
    coordinator.reload()
}

fn config_error_message(error: &ConfigCoordinatorError) -> String {
    match config_warning_message(error.warnings()) {
        Some(warning) => format!("{error}. {warning}"),
        None => error.to_string(),
    }
}

fn config_warning_message(warnings: &[ConfigWarning]) -> Option<String> {
    let messages = warnings
        .iter()
        .map(|warning| match warning {
            ConfigWarning::StartWithWindows {
                desired,
                confirmed,
                message,
            } => {
                let action = if *desired { "enabled" } else { "disabled" };
                let outcome = if *confirmed == Some(*desired) {
                    format!("Start with Windows was {action} with a warning")
                } else {
                    format!("Start with Windows could not be {action}")
                };
                let confirmed = confirmed.map_or_else(
                    || "The applied state could not be confirmed".to_string(),
                    |confirmed| {
                        format!(
                            "The confirmed setting is {}",
                            if confirmed { "enabled" } else { "disabled" }
                        )
                    },
                );
                let detail = message
                    .as_deref()
                    .map_or_else(String::new, |message| format!(": {message}"));
                format!("{outcome}. {confirmed}{detail}")
            }
            ConfigWarning::StartWithWindowsRollback {
                target,
                attempted,
                confirmed,
                message,
            } => {
                let target = if *target { "enabled" } else { "disabled" };
                let attempt = if *attempted {
                    format!("Rollback to {target} was attempted")
                } else {
                    format!("The setting remained {target}")
                };
                let confirmed = confirmed.map_or_else(
                    || "but the applied state could not be confirmed".to_string(),
                    |confirmed| {
                        format!(
                            "and the confirmed setting is {}",
                            if confirmed { "enabled" } else { "disabled" }
                        )
                    },
                );
                let detail = message
                    .as_deref()
                    .map_or_else(String::new, |message| format!(": {message}"));
                format!("{attempt} {confirmed}{detail}")
            }
        })
        .collect::<Vec<_>>();
    (!messages.is_empty()).then(|| messages.join(" "))
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

#[cfg(test)]
#[path = "settings_view/tests.rs"]
mod tests;
