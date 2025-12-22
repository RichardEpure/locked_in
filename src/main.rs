mod components;
mod config;
mod hid;
mod win;

use dioxus::{
    desktop::{
        Config, WindowBuilder,
        trayicon::{
            Icon, TrayIconBuilder,
            menu::{Menu, MenuItem},
        },
        use_tray_menu_event_handler,
    },
    prelude::*,
};

use crate::components::{dialog::Dialog, hid_devices::HidDevices};

const FAVICON: Asset = asset!("/assets/favicon.ico");
const MAIN_CSS: Asset = asset!("/assets/styles/main.css");
const HEADER_SVG: Asset = asset!("/assets/header.svg");

static FOCUSED_WINDOW_SIGNAL: GlobalSignal<win::WindowMetadata> =
    Signal::global(win::get_focused_window);

static CONFIG_SIGNAL: GlobalSignal<config::Config> = Signal::global(config::init_config);

fn main() {
    let _foreground_hook = win::start_foreground_hook();

    dioxus::LaunchBuilder::desktop()
        .with_cfg(
            Config::new()
                .with_window(
                    WindowBuilder::new()
                        .with_title("Locked In")
                        .with_resizable(true),
                )
                .with_close_behaviour(dioxus::desktop::WindowCloseBehaviour::WindowHides),
        )
        .launch(App);
}

#[component]
fn App() -> Element {
    let icon = Icon::from_path("assets/favicon.ico", None).unwrap();
    let menu = Menu::new();
    let menu_item_quit = MenuItem::with_id("quit", "Quit", true, None);
    let menu_item_toggle = MenuItem::with_id("toggle", "Toggle", true, None);
    menu.append_items(&[&menu_item_quit, &menu_item_toggle])
        .unwrap();

    let builder = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_icon(icon);

    provide_context(builder.build().expect("Failed to build tray icon"));

    {
        use_tray_menu_event_handler(move |event| match event.id.0.as_str() {
            "quit" => {
                std::process::exit(0);
            }
            "toggle" => {
                println!("Toggle clicked");
            }
            _ => {}
        });
    }

    use_future(move || async move {
        let mut rx = win::FOCUSED_WINDOW_TX.subscribe();
        loop {
            if rx.changed().await.is_err() {
                break;
            }
            let latest = rx.borrow().clone();
            *FOCUSED_WINDOW_SIGNAL.write() = latest.clone();
        }
    });

    rsx! {
        document::Link { rel: "icon", href: FAVICON }
        document::Link { rel: "stylesheet", href: MAIN_CSS }
        DeviceList {}
    }
}

#[component]
fn Test() -> Element {
    rsx! {
        h1 { "Test Component" }
    }
}

#[component]
fn DeviceList() -> Element {
    let mut hid_devices = use_signal(hid::get_devices);

    let focused_window_title = FOCUSED_WINDOW_SIGNAL
        .read()
        .title
        .clone()
        .unwrap_or("null".to_string());
    let focused_window_class = FOCUSED_WINDOW_SIGNAL
        .read()
        .class
        .clone()
        .unwrap_or("null".to_string());

    let mut show_add_device_modal = use_signal(|| false);

    rsx! {
        h2 { "Window data: {focused_window_title} - {focused_window_class}" }
        h1 {
            id: "device-list",
            "Device List"
        }
        button {
            onclick: move |_| {
                show_add_device_modal.set(true);
            },
            "Add Device"
        }
        button {
            onclick: move |_| {
                hid_devices.set(hid::get_devices());
            },
            "Refresh Device List"
        }
        ul {
            for device in CONFIG_SIGNAL.read().devices.iter() {
                li {
                    "{device.name}"
                }
            }
        }
        if *show_add_device_modal.read() {
            Dialog {
                open: *show_add_device_modal.read(),
                title: "Add Device".to_string(),
                hide_buttons: true,
                on_ok: move |_| {
                    show_add_device_modal.set(false);
                },
                on_cancel: move |_| {
                    show_add_device_modal.set(false);
                },
                HidDevices {
                    on_select: move |device: hid::HidMetadata| {
                        show_add_device_modal.set(false);
                        println!("Selected device: {:?}", device);
                    }
                },
            }
        }
    }
}
