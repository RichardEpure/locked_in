mod app_log;
mod automation_runtime;
mod components;
mod config;
mod config_runtime_bridge;
mod focused_window;
mod hid;
mod win;

use std::{
    io::Write,
    path::Path,
    sync::{Arc, LazyLock, OnceLock},
};

use anyhow::{Context, Result, anyhow};
use dioxus::{
    desktop::{Config as DesktopConfig, LogicalSize, WindowBuilder},
    prelude::*,
};

pub static FOCUSED_WINDOW_SIGNAL: GlobalSignal<win::WindowMetadata> =
    Signal::global(win::get_focused_window);
pub static CAPTURED_WINDOW_SIGNAL: GlobalSignal<Option<CapturedWindow>> = Signal::global(|| None);
pub static CAPTURE_ARMED_SIGNAL: GlobalSignal<bool> = Signal::global(|| false);
pub static CAPTURE_GENERATION_SIGNAL: GlobalSignal<u64> = Signal::global(|| 0);
pub static CAPTURE_TARGET_SIGNAL: GlobalSignal<Option<CaptureTarget>> = Signal::global(|| None);
pub static CONFIG_REVISION_SIGNAL: GlobalSignal<u64> =
    Signal::global(|| config_bootstrap().map_or(0, |bootstrap| bootstrap.revision));
pub static DIRTY_EDITOR_SIGNAL: GlobalSignal<Option<String>> = Signal::global(|| None);
pub static UNSAVED_ENTITY_SIGNAL: GlobalSignal<Option<String>> = Signal::global(|| None);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureTarget {
    pub(crate) automation_id: String,
    pub(crate) case_id: String,
    pub(crate) exception: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedWindow {
    pub(crate) generation: u64,
    pub(crate) target: Option<CaptureTarget>,
    pub(crate) window: win::WindowMetadata,
}

impl CapturedWindow {
    pub(crate) fn belongs_to(&self, generation: u64, target: &Option<CaptureTarget>) -> bool {
        self.generation == generation && &self.target == target
    }
}

impl CaptureTarget {
    pub fn new(automation_id: String, case_id: String, exception: bool) -> Self {
        Self {
            automation_id,
            case_id,
            exception,
        }
    }
}

pub fn arm_capture(target: Option<CaptureTarget>) {
    let generation = CAPTURE_GENERATION_SIGNAL
        .read()
        .checked_add(1)
        .expect("capture generation overflow");
    *CAPTURE_GENERATION_SIGNAL.write() = generation;
    *CAPTURED_WINDOW_SIGNAL.write() = None;
    *CAPTURE_TARGET_SIGNAL.write() = target;
    *CAPTURE_ARMED_SIGNAL.write() = true;
    spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
        if *CAPTURE_ARMED_SIGNAL.read() && *CAPTURE_GENERATION_SIGNAL.read() == generation {
            cancel_capture();
        }
    });
}

pub fn cancel_capture() {
    let generation = CAPTURE_GENERATION_SIGNAL
        .read()
        .checked_add(1)
        .expect("capture generation overflow");
    *CAPTURE_GENERATION_SIGNAL.write() = generation;
    *CAPTURE_ARMED_SIGNAL.write() = false;
    *CAPTURE_TARGET_SIGNAL.write() = None;
    *CAPTURED_WINDOW_SIGNAL.write() = None;
}

struct ConfigBootstrap {
    ui: config::Config,
    revision: u64,
    error: Option<String>,
}

struct InitialConfiguration {
    coordinator: Option<Arc<config::ConfigCoordinator>>,
    publication: Option<Arc<config::PublishedConfig>>,
    bootstrap: ConfigBootstrap,
}

struct PreparedApplicationPaths {
    paths: config::ApplicationPaths,
    bootstrap_error: Option<String>,
}

static CONFIG_BOOTSTRAP: OnceLock<ConfigBootstrap> = OnceLock::new();

fn config_bootstrap() -> Option<&'static ConfigBootstrap> {
    CONFIG_BOOTSTRAP.get()
}

pub static CONFIG_SIGNAL: GlobalSignal<config::Config> = Signal::global(|| {
    config_bootstrap().map_or_else(config::Config::default, |value| value.ui.clone())
});
pub static CONFIG_LOAD_ERROR: LazyLock<Option<String>> =
    LazyLock::new(|| config_bootstrap().and_then(|value| value.error.clone()));

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

struct WindowsStartWithWindows;

impl config::StartWithWindows for WindowsStartWithWindows {
    fn reconcile(&self, desired: bool) -> config::StartWithWindowsOutcome {
        reconcile_start_with_windows(
            desired,
            win::set_start_with_windows,
            win::start_with_windows_enabled,
        )
    }
}

fn reconcile_start_with_windows(
    desired: bool,
    apply: impl FnOnce(bool) -> Result<()>,
    inspect: impl FnOnce() -> Result<bool>,
) -> config::StartWithWindowsOutcome {
    match apply(desired) {
        Ok(()) => config::StartWithWindowsOutcome::confirmed(desired),
        Err(apply_error) => match inspect() {
            Ok(confirmed) => config::StartWithWindowsOutcome::warning(
                confirmed,
                format!("Windows startup registration failed: {apply_error:#}"),
            ),
            Err(inspect_error) => config::StartWithWindowsOutcome::unconfirmed(format!(
                "Windows startup registration failed: {apply_error:#}; the applied state could not be confirmed: {inspect_error:#}"
            )),
        },
    }
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
    start_with_windows: Arc<dyn config::StartWithWindows>,
) -> InitialConfiguration {
    match config::ConfigCoordinator::initial_load(store, start_with_windows) {
        Ok(coordinator) => {
            let coordinator = Arc::new(coordinator);
            let publication = coordinator.current();
            InitialConfiguration {
                coordinator: Some(coordinator),
                publication: Some(publication.clone()),
                bootstrap: ConfigBootstrap {
                    ui: publication.editable().as_ref().clone(),
                    revision: publication.revision(),
                    error: None,
                },
            }
        }
        Err(error) => InitialConfiguration {
            coordinator: None,
            publication: None,
            bootstrap: ConfigBootstrap {
                ui: config::Config::default(),
                revision: 0,
                error: Some(format!("Configuration could not be loaded: {error}")),
            },
        },
    }
}

fn initialize_configuration(
    prepared: &PreparedApplicationPaths,
    load: impl FnOnce(&config::ApplicationPaths) -> InitialConfiguration,
) -> InitialConfiguration {
    match &prepared.bootstrap_error {
        Some(error) => InitialConfiguration {
            coordinator: None,
            publication: None,
            bootstrap: ConfigBootstrap {
                ui: config::Config::default(),
                revision: 0,
                error: Some(error.clone()),
            },
        },
        None => load(&prepared.paths),
    }
}

fn main() {
    let visibility_override = std::env::var_os("LOCKED_IN_FORCE_VISIBLE").is_some();
    let _instance = if cfg!(debug_assertions) && visibility_override {
        None
    } else {
        match win::claim_single_instance() {
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
        if let Err(error) = config::initialize_facade(paths.clone(), store.clone()) {
            InitialConfiguration {
                coordinator: None,
                publication: None,
                bootstrap: ConfigBootstrap {
                    ui: config::Config::default(),
                    revision: 0,
                    error: Some(format!("Application bootstrap failed: {error}")),
                },
            }
        } else {
            load_initial_configuration(store, Arc::new(WindowsStartWithWindows))
        }
    });
    let coordinator = initial.coordinator;
    let publication = initial.publication;
    let bootstrap_error = initial.bootstrap.error.clone();
    let settings = initial.bootstrap.ui.settings.clone();
    assert!(
        CONFIG_BOOTSTRAP.set(initial.bootstrap).is_ok(),
        "configuration bootstrap must be initialized once"
    );
    app_log::set_level(settings.log_level);
    app_log::write("application started");
    if let Some(error) = &bootstrap_error {
        app_log::write_error(format!("application bootstrap error: {error}"));
    }

    let publication_subscription = coordinator.as_ref().map(|value| value.subscribe());
    let focus_events = win::subscribe_foreground_observations();
    let (foreground_hook, focus_source) = match win::start_foreground_hook() {
        Ok(hook) => (Some(hook), automation_runtime::FocusSourceState::Available),
        Err(error) => {
            let message = format!("foreground hook failed: {error:#}");
            app_log::write_error(&message);
            (
                None,
                automation_runtime::FocusSourceState::Unavailable(message),
            )
        }
    };
    let active = publication.as_ref().map(|value| value.active().clone());
    let (runtime, runtime_owner) = match automation_runtime::AutomationRuntime::start_active(
        active,
        focus_events,
        focus_source,
        hid::SystemHidBackend::new(),
    ) {
        Ok(runtime) => runtime,
        Err(error) => {
            app_log::write_error(format!("automation runtime failed to start: {error:#}"));
            return;
        }
    };
    let runtime_bridge = match publication_subscription.clone() {
        Some(subscription) => {
            match config_runtime_bridge::ConfigRuntimeBridge::start(runtime.clone(), subscription) {
                Ok(bridge) => Some(bridge),
                Err(error) => {
                    app_log::write_error(format!(
                        "configuration runtime bridge failed to start: {error:#}"
                    ));
                    runtime_owner.shutdown_and_join(std::time::Duration::from_secs(2));
                    return;
                }
            }
        }
        None => None,
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
                    bootstrap_error.as_deref(),
                    &settings,
                )),
        )
        .with_menu(None)
        .with_close_behaviour(dioxus::desktop::WindowCloseBehaviour::WindowCloses)
        .with_tray_icon_show_window_on_click(false);
    desktop_config = desktop_config.with_data_directory(paths.webview_data_directory());

    dioxus::LaunchBuilder::desktop()
        .with_context(runtime)
        .with_context(coordinator.clone())
        .with_context(publication_subscription.clone())
        .with_context(paths.clone())
        .with_cfg(desktop_config)
        .launch(components::App);
    drop(foreground_hook);
    if let Some(runtime_bridge) = runtime_bridge {
        runtime_bridge.shutdown_and_join();
    }
    runtime_owner.shutdown_and_join(std::time::Duration::from_secs(2));
    drop(publication_subscription);
    drop(coordinator);
}

#[cfg(test)]
#[path = "main/tests.rs"]
mod tests;
