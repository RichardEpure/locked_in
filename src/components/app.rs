use dioxus::{
    desktop::{
        trayicon::{
            Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent,
            menu::{Menu, MenuItem},
        },
        use_muda_event_handler, use_tray_icon_event_handler, use_window,
    },
    prelude::*,
};

use crate::{
    CONFIG_SIGNAL, FOCUSED_WINDOW_SIGNAL, HID_CACHE_REVISION_SIGNAL, app_log, arm_capture, hid, win,
};

use super::{capture_shortcut::CaptureShortcut, workspace::Workspace};

const FAVICON: Asset = asset!("/assets/favicon.ico");
const MAIN_CSS: Asset = asset!("/assets/styles/main.css");

#[component]
pub(crate) fn App() -> Element {
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
