mod app_log;
mod components;
mod config;
mod hid;
mod win;

use std::{
    io::Write,
    sync::{
        LazyLock,
        atomic::{AtomicBool, Ordering},
    },
};

use dioxus::{
    desktop::{
        Config as DesktopConfig, HotKeyState, LogicalSize, WindowBuilder,
        trayicon::{
            Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent,
            menu::{Menu, MenuItem},
        },
        use_global_shortcut, use_muda_event_handler, use_tray_icon_event_handler, use_window,
    },
    prelude::*,
};

use crate::components::workspace::Workspace;

const FAVICON: Asset = asset!("/assets/favicon.ico");
const MAIN_CSS: Asset = asset!("/assets/styles/main.css");

pub static FOCUSED_WINDOW_SIGNAL: GlobalSignal<win::WindowMetadata> =
    Signal::global(win::get_focused_window);
pub static CAPTURED_WINDOW_SIGNAL: GlobalSignal<Option<win::WindowMetadata>> =
    Signal::global(|| None);
pub static CAPTURE_ARMED_SIGNAL: GlobalSignal<bool> = Signal::global(|| false);
pub static CAPTURE_GENERATION_SIGNAL: GlobalSignal<u64> = Signal::global(|| 0);
pub static CAPTURE_TARGET_SIGNAL: GlobalSignal<Option<CaptureTarget>> = Signal::global(|| None);
pub static CONFIG_REVISION_SIGNAL: GlobalSignal<u64> = Signal::global(|| 0);
pub static HID_CACHE_REVISION_SIGNAL: GlobalSignal<u64> = Signal::global(|| 0);
pub static DIRTY_EDITOR_SIGNAL: GlobalSignal<Option<String>> = Signal::global(|| None);
pub static UNSAVED_ENTITY_SIGNAL: GlobalSignal<Option<String>> = Signal::global(|| None);
pub static SERVICE_READY: AtomicBool = AtomicBool::new(false);

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
    *CAPTURE_TARGET_SIGNAL.write() = target;
    *CAPTURE_ARMED_SIGNAL.write() = true;
    *CAPTURE_GENERATION_SIGNAL.write() += 1;
    let generation = *CAPTURE_GENERATION_SIGNAL.read();
    spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
        if *CAPTURE_GENERATION_SIGNAL.read() == generation {
            *CAPTURE_ARMED_SIGNAL.write() = false;
            *CAPTURE_TARGET_SIGNAL.write() = None;
        }
    });
}

pub fn cancel_capture() {
    *CAPTURE_ARMED_SIGNAL.write() = false;
    *CAPTURE_TARGET_SIGNAL.write() = None;
    *CAPTURE_GENERATION_SIGNAL.write() += 1;
}

pub static CONFIG_SIGNAL: GlobalSignal<config::Config> = Signal::global(|| {
    config::Config::load().unwrap_or_else(|error| {
        app_log::write_error(format!("configuration load failed: {error:#}"));
        config::Config::default()
    })
});
pub static CONFIG_LOAD_ERROR: LazyLock<Option<String>> = LazyLock::new(|| {
    config::Config::load()
        .err()
        .map(|error| format!("{error:#}"))
});

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
    let _foreground_hook = match win::start_foreground_hook() {
        Ok(hook) => {
            SERVICE_READY.store(true, Ordering::Relaxed);
            Some(hook)
        }
        Err(error) => {
            app_log::write_error(format!("foreground hook failed: {error:#}"));
            None
        }
    };
    let data_directory = config::data_directory().unwrap_or_else(|error| {
        eprintln!("data directory unavailable: {error:#}");
        std::env::temp_dir().join("LockedIn")
    });
    let _ = std::fs::create_dir_all(&data_directory);
    install_panic_log(data_directory.clone());
    app_log::write("application started");
    let settings = config::Config::load().unwrap_or_default().settings;
    app_log::set_level(settings.log_level);
    let close_behaviour = if settings.close_to_tray {
        dioxus::desktop::WindowCloseBehaviour::WindowHides
    } else {
        dioxus::desktop::WindowCloseBehaviour::WindowCloses
    };

    dioxus::LaunchBuilder::desktop()
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
        .launch(App);
}

#[component]
fn App() -> Element {
    let window = use_window();
    let menu = Menu::new();
    let open_item = MenuItem::with_id("open", "Open Locked In", true, None);
    let capture_item = MenuItem::with_id("capture", "Capture focused window (F3)", true, None);
    let quit_item = MenuItem::with_id("quit", "Quit", true, None);
    let menu_ready = menu
        .append_items(&[&open_item, &capture_item, &quit_item])
        .is_ok();

    let mut builder = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false);
    if let Ok(path) = dioxus::asset_resolver::asset_path(FAVICON)
        && let Ok(icon) = Icon::from_path(path, None)
    {
        builder = builder.with_icon(icon);
    }
    if menu_ready && let Ok(tray) = builder.build() {
        provide_context(tray);
    } else {
        app_log::write("tray icon could not be created");
        window.set_close_behavior(dioxus::desktop::WindowCloseBehaviour::WindowCloses);
        window.set_visible(true);
    }

    use_muda_event_handler({
        let window = window.clone();
        move |event| match event.id.0.as_str() {
            "quit" => std::process::exit(0),
            "open" => {
                window.set_visible(true);
                window.set_minimized(false);
                window.set_focus();
            }
            "capture" => {
                arm_capture(None);
                app_log::write("focused-window capture armed");
            }
            _ => {}
        }
    });

    use_tray_icon_event_handler({
        let window = window.clone();
        move |event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                let visible = window.is_visible();
                window.set_visible(!visible);
                if !visible {
                    window.set_minimized(false);
                    window.set_focus();
                }
            }
        }
    });

    use_future(move || async move {
        match tokio::task::spawn_blocking(hid::initialize_device_cache).await {
            Ok(Ok(())) => *HID_CACHE_REVISION_SIGNAL.write() += 1,
            Ok(Err(error)) => app_log::write_error(format!("HID discovery failed: {error:#}")),
            Err(error) => app_log::write_error(format!("HID discovery task failed: {error}")),
        }
    });

    use_future(move || async move {
        let mut receiver = win::FOCUSED_WINDOW_TX.subscribe();
        while receiver.changed().await.is_ok() {
            let focused = receiver.borrow().clone();
            *FOCUSED_WINDOW_SIGNAL.write() = focused.clone();
            let config = CONFIG_SIGNAL.read();
            for evaluated in config.evaluate_window(&focused) {
                for device in evaluated.devices {
                    match device.send_report(&evaluated.action.report) {
                        Ok(_) => app_log::write(format!(
                            "{} / {} sent {} bytes to {}",
                            evaluated.automation_name,
                            evaluated.case_name,
                            evaluated.action.report.len(),
                            device.name
                        )),
                        Err(error) => app_log::write_error(format!(
                            "{} / {} failed for {}: {error:#}",
                            evaluated.automation_name, evaluated.case_name, device.name
                        )),
                    }
                }
            }
        }
    });

    rsx! {
        document::Link { rel: "icon", href: FAVICON }
        document::Link { rel: "stylesheet", href: MAIN_CSS }
        CaptureShortcut {}
        Workspace {}
    }
}

#[component]
fn CaptureShortcut() -> Element {
    if !*CAPTURE_ARMED_SIGNAL.read() {
        return rsx! {};
    }
    rsx! { ArmedCaptureShortcut {} }
}

#[component]
fn ArmedCaptureShortcut() -> Element {
    let window = use_window();
    let _shortcut = use_global_shortcut(KeyCode::F3, move |state| {
        if state != HotKeyState::Pressed {
            return;
        }
        *CAPTURED_WINDOW_SIGNAL.write() = Some(FOCUSED_WINDOW_SIGNAL.read().clone());
        *CAPTURE_ARMED_SIGNAL.write() = false;
        window.set_visible(true);
        window.set_minimized(false);
        window.set_focus();
        app_log::write("focused window captured");
    });
    rsx! {}
}
