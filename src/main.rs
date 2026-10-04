mod app_log;
mod application_lifecycle;
mod automation_runtime;
mod components;
mod config;
mod event;
mod focused_window;
mod hid;
mod platform;

use std::{io::Write, path::Path, sync::Arc};

use anyhow::{Context, Result, anyhow};
use dioxus::{
    desktop::{Config as DesktopConfig, LogicalSize, WindowBuilder},
    prelude::*,
};

pub static FOCUSED_WINDOW_SIGNAL: GlobalSignal<focused_window::FocusedWindow> =
    Signal::global(|| platform::foreground::get().current().window);
pub static DIRTY_EDITOR_SIGNAL: GlobalSignal<Option<String>> = Signal::global(|| None);

#[derive(Debug, Clone)]
pub(crate) struct ConfigurationLoadError {
    pub(crate) message: String,
    pub(crate) config_path_available: bool,
}

struct PreparedApplicationPaths {
    paths: config::ApplicationPaths,
    bootstrap_error: Option<String>,
}

fn install_panic_log(log_path: std::path::PathBuf) {
    std::panic::set_hook(Box::new(move |info| {
        let bt = std::backtrace::Backtrace::force_capture();
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            let _ = writeln!(file, "PANIC: {info}\nBACKTRACE:\n{bt}\n---\n");
        }
    }));
}

fn prepare_application_paths() -> Result<PreparedApplicationPaths> {
    let preferred =
        config::resolve_application_paths().context("application paths are unavailable");
    let fallback = config::ApplicationPaths::from_data_root(std::env::temp_dir().join(format!(
        "LockedIn-bootstrap-fallback-{}",
        std::process::id()
    )));
    prepare_application_paths_from(preferred, fallback)
}

fn prepare_application_paths_from(
    preferred: Result<config::ApplicationPaths>,
    fallback: config::ApplicationPaths,
) -> Result<PreparedApplicationPaths> {
    match preferred.and_then(|paths| {
        prepare_directories(&paths)?;
        Ok(paths)
    }) {
        Ok(paths) => Ok(PreparedApplicationPaths {
            paths,
            bootstrap_error: None,
        }),
        Err(preferred_error) => {
            prepare_directories(&fallback).map_err(|fallback_error| {
                anyhow!(
                    "preferred application root failed: {preferred_error:#}; temporary fallback root {} also failed: {fallback_error:#}",
                    fallback.data_root().display()
                )
            })?;
            Ok(PreparedApplicationPaths {
                bootstrap_error: Some(format!(
                    "Application data root failed: {preferred_error:#}. Using temporary diagnostics and WebView root {}. Configuration was not loaded.",
                    fallback.data_root().display()
                )),
                paths: fallback,
            })
        }
    }
}

fn prepare_directories(paths: &config::ApplicationPaths) -> Result<()> {
    create_directory(paths.data_root())?;
    create_directory(&paths.log_directory())?;
    create_directory(&paths.webview_data_directory())?;
    Ok(())
}

fn create_directory(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)
        .with_context(|| format!("failed to create application directory {}", path.display()))
}

fn initial_window_visible(
    visibility_override: bool,
    bootstrap_error: Option<&str>,
    settings: &config::Settings,
) -> bool {
    visibility_override || bootstrap_error.is_some() || !settings.start_minimized
}

fn load_initial_configuration(
    store: Arc<config::ConfigStore>,
    launch_at_login: Arc<dyn platform::autostart::LaunchAtLogin>,
) -> Result<Arc<config::ConfigCoordinator>, ConfigurationLoadError> {
    config::ConfigCoordinator::initial_load(store, launch_at_login)
        .map(Arc::new)
        .map_err(|error| ConfigurationLoadError {
            message: format!("Configuration could not be loaded: {error}"),
            config_path_available: true,
        })
}

fn initialize_configuration(
    prepared: &PreparedApplicationPaths,
    load: impl FnOnce(
        &config::ApplicationPaths,
    ) -> Result<Arc<config::ConfigCoordinator>, ConfigurationLoadError>,
) -> Result<Arc<config::ConfigCoordinator>, ConfigurationLoadError> {
    match &prepared.bootstrap_error {
        Some(error) => Err(ConfigurationLoadError {
            message: error.clone(),
            config_path_available: false,
        }),
        None => load(&prepared.paths),
    }
}

fn main() {
    let visibility_override = std::env::var_os("LOCKED_IN_FORCE_VISIBLE").is_some();
    let _instance = if cfg!(debug_assertions) && visibility_override {
        None
    } else {
        match platform::instance::claim() {
            Ok(Some(instance)) => Some(instance),
            Ok(None) => return,
            Err(error) => {
                eprintln!("single-instance setup failed: {error:#}");
                return;
            }
        }
    };

    let prepared_paths = match prepare_application_paths() {
        Ok(prepared) => prepared,
        Err(error) => {
            eprintln!("application startup aborted: {error:#}");
            return;
        }
    };
    let paths = prepared_paths.paths.clone();
    install_panic_log(paths.panic_log_path());
    if let Err(path) = app_log::initialize(paths.log_directory()) {
        eprintln!(
            "application startup aborted because logging was already initialized at {}",
            path.display()
        );
        return;
    }
    if let Some(error) = &prepared_paths.bootstrap_error {
        eprintln!("application bootstrap fallback: {error}");
    }

    let initial = initialize_configuration(&prepared_paths, |paths| {
        let store = Arc::new(config::ConfigStore::new(paths.config_path()));
        load_initial_configuration(store, Arc::new(platform::autostart::SystemLaunchAtLogin))
    });
    let (coordinator, configuration_load_error) = match initial {
        Ok(coordinator) => (Some(coordinator), None),
        Err(error) => (None, Some(error)),
    };
    let publication = coordinator.as_ref().map(|value| value.current());
    let settings = publication
        .as_ref()
        .map_or_else(config::Settings::default, |value| {
            value.editable().settings.clone()
        });
    app_log::set_level(settings.log_level);
    app_log::write("application started");
    if let Some(error) = &configuration_load_error {
        app_log::write_error(format!("application bootstrap error: {}", error.message));
    }

    let publication_subscription = coordinator.as_ref().map(|value| value.subscribe());
    let focus_events = platform::foreground::get().subscribe();
    let (foreground_monitor, focus_source) = match platform::foreground::start() {
        Ok(monitor) => (
            Some(monitor),
            automation_runtime::EventSourceState::Available,
        ),
        Err(error) => {
            let message = format!("foreground monitoring failed: {error:#}");
            app_log::write_error(&message);
            (
                None,
                automation_runtime::EventSourceState::Unavailable(message),
            )
        }
    };
    let (runtime, runtime_owner) = match automation_runtime::AutomationRuntime::start(
        publication_subscription.clone(),
        automation_runtime::RuntimeInputs {
            focused_window: automation_runtime::FocusInput::new(focus_events, focus_source),
        },
        hid::SystemHidBackend::new(),
    ) {
        Ok(runtime) => runtime,
        Err(error) => {
            app_log::write_error(format!("automation runtime failed to start: {error:#}"));
            return;
        }
    };
    let mut desktop_config = DesktopConfig::new()
        .with_window(
            WindowBuilder::new()
                .with_title("Locked In")
                .with_inner_size(LogicalSize::new(1280.0, 800.0))
                .with_min_inner_size(LogicalSize::new(1000.0, 650.0))
                .with_resizable(true)
                .with_visible(initial_window_visible(
                    visibility_override,
                    configuration_load_error
                        .as_ref()
                        .map(|error| error.message.as_str()),
                    &settings,
                )),
        )
        .with_menu(None)
        // App intercepts exit requests and keeps its completion task alive until
        // the backend join finishes. Close-to-tray uses the same hide behavior.
        .with_close_behaviour(dioxus::desktop::WindowCloseBehaviour::WindowHides)
        .with_tray_icon_show_window_on_click(false);
    desktop_config = desktop_config.with_data_directory(paths.webview_data_directory());
    let mut foreground_monitor = foreground_monitor;
    desktop_config = desktop_config.with_custom_event_handler(move |event, _| {
        if matches!(event, dioxus::desktop::tao::event::Event::LoopDestroyed) {
            // Release native observation on its registering desktop thread.
            drop(foreground_monitor.take());
        }
    });
    let lifecycle = Arc::new(application_lifecycle::ApplicationLifecycle::new(
        runtime_owner,
    ));

    dioxus::LaunchBuilder::desktop()
        .with_context(runtime)
        .with_context(lifecycle)
        .with_context(coordinator.clone())
        .with_context(publication_subscription.clone())
        .with_context(paths.clone())
        .with_context(configuration_load_error)
        .with_cfg(desktop_config)
        .launch(components::App);
}

#[cfg(test)]
#[path = "main/tests.rs"]
mod tests;
