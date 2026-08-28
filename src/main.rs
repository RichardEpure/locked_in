mod app_log;
mod automation_runtime;
mod components;
mod config;
mod focused_window;
mod hid;
mod win;

use std::{io::Write, sync::LazyLock};

use dioxus::{
    desktop::{Config as DesktopConfig, LogicalSize, WindowBuilder},
    prelude::*,
};

pub static FOCUSED_WINDOW_SIGNAL: GlobalSignal<win::WindowMetadata> =
    Signal::global(win::get_focused_window);
pub static CAPTURED_WINDOW_SIGNAL: GlobalSignal<Option<win::WindowMetadata>> =
    Signal::global(|| None);
pub static CAPTURE_ARMED_SIGNAL: GlobalSignal<bool> = Signal::global(|| false);
pub static CAPTURE_GENERATION_SIGNAL: GlobalSignal<u64> = Signal::global(|| 0);
pub static CAPTURE_TARGET_SIGNAL: GlobalSignal<Option<CaptureTarget>> = Signal::global(|| None);
pub static CONFIG_REVISION_SIGNAL: GlobalSignal<u64> = Signal::global(|| 0);
pub static DIRTY_EDITOR_SIGNAL: GlobalSignal<Option<String>> = Signal::global(|| None);
pub static UNSAVED_ENTITY_SIGNAL: GlobalSignal<Option<String>> = Signal::global(|| None);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureTarget {
    pub(crate) automation_id: String,
    pub(crate) case_id: String,
    pub(crate) exception: bool,
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
    *CAPTURED_WINDOW_SIGNAL.write() = None;
    *CAPTURE_TARGET_SIGNAL.write() = target;
    *CAPTURE_ARMED_SIGNAL.write() = true;
    *CAPTURE_GENERATION_SIGNAL.write() += 1;
    let generation = *CAPTURE_GENERATION_SIGNAL.read();
    spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
        if *CAPTURE_GENERATION_SIGNAL.read() == generation {
            cancel_capture();
        }
    });
}

pub fn cancel_capture() {
    *CAPTURE_ARMED_SIGNAL.write() = false;
    *CAPTURE_TARGET_SIGNAL.write() = None;
    *CAPTURED_WINDOW_SIGNAL.write() = None;
    *CAPTURE_GENERATION_SIGNAL.write() += 1;
}

struct ConfigBootstrap {
    active: Option<config::Config>,
    ui: config::Config,
    error: Option<String>,
}

static CONFIG_BOOTSTRAP: LazyLock<ConfigBootstrap> = LazyLock::new(|| match config::load() {
    Ok(config) => ConfigBootstrap {
        active: Some(config.clone()),
        ui: config,
        error: None,
    },
    Err(error) => ConfigBootstrap {
        active: None,
        ui: config::Config::default(),
        error: Some(format!("{error:#}")),
    },
});

pub static CONFIG_SIGNAL: GlobalSignal<config::Config> =
    Signal::global(|| CONFIG_BOOTSTRAP.ui.clone());
pub static CONFIG_LOAD_ERROR: LazyLock<Option<String>> =
    LazyLock::new(|| CONFIG_BOOTSTRAP.error.clone());

fn install_panic_log(mut log_path: std::path::PathBuf) {
    log_path.push("panic.log");
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

fn main() {
    let force_visible = std::env::var_os("LOCKED_IN_FORCE_VISIBLE").is_some();
    let _instance = if cfg!(debug_assertions) && force_visible {
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
    let data_directory = config::data_directory().unwrap_or_else(|error| {
        eprintln!("data directory unavailable: {error:#}");
        std::env::temp_dir().join("LockedIn")
    });
    let _ = std::fs::create_dir_all(&data_directory);
    install_panic_log(data_directory.clone());
    let settings = CONFIG_BOOTSTRAP.ui.settings.clone();
    app_log::set_level(settings.log_level);
    app_log::write("application started");
    if let Some(error) = &CONFIG_BOOTSTRAP.error {
        app_log::write_error(format!("configuration load failed: {error}"));
    }
    let focus_events = win::subscribe_foreground_observations();
    let (_foreground_hook, focus_source) = match win::start_foreground_hook() {
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
    let (runtime, runtime_owner) = match automation_runtime::AutomationRuntime::start(
        CONFIG_BOOTSTRAP.active.clone(),
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
    let close_behaviour = if settings.close_to_tray {
        dioxus::desktop::WindowCloseBehaviour::WindowHides
    } else {
        dioxus::desktop::WindowCloseBehaviour::WindowCloses
    };

    dioxus::LaunchBuilder::desktop()
        .with_context(runtime)
        .with_cfg(
            DesktopConfig::new()
                .with_window(
                    WindowBuilder::new()
                        .with_title("Locked In")
                        .with_inner_size(LogicalSize::new(1280.0, 800.0))
                        .with_min_inner_size(LogicalSize::new(1000.0, 650.0))
                        .with_resizable(true)
                        .with_visible(force_visible || !settings.start_minimized),
                )
                .with_menu(None)
                .with_close_behaviour(close_behaviour)
                .with_tray_icon_show_window_on_click(false)
                .with_data_directory(data_directory.join("webview")),
        )
        .launch(components::App);
    runtime_owner.shutdown_and_join(std::time::Duration::from_secs(2));
}
