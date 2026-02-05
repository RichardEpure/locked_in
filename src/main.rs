mod components;
mod config;
mod hid;
mod win;

use std::io::Write;

use dioxus::{
    desktop::{
        Config, WindowBuilder,
        trayicon::{
            Icon, TrayIconBuilder,
            menu::{Menu, MenuItem},
        },
        use_muda_event_handler,
    },
    prelude::*,
};

use crate::components::{
    dialog::Dialog,
    edit_rule::EditRule,
    events::{
        capture_focused_window::CaptureFocusedWindow,
        capture_focused_window_shortcut::CaptureFocusedWindowShortcut,
    },
    rules::Rules,
};

const FAVICON: Asset = asset!("/assets/favicon.ico");
const MAIN_CSS: Asset = asset!("/assets/styles/main.css");
const HEADER_SVG: Asset = asset!("/assets/header.svg");

static FOCUSED_WINDOW_SIGNAL: GlobalSignal<win::WindowMetadata> =
    Signal::global(win::get_focused_window);

pub static CONFIG_SIGNAL: GlobalSignal<config::Config> =
    Signal::global(|| config::Config::load().expect("Failed to load config"));

fn install_panic_log(mut log_path: std::path::PathBuf) {
    log_path.push("panic.log");

    std::panic::set_hook(Box::new(move |info| {
        let bt = std::backtrace::Backtrace::force_capture();
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .ok();

        if let Some(f) = f.as_mut() {
            let _ = writeln!(f, "PANIC: {info}\nBACKTRACE:\n{bt}\n---\n");
        }
    }));
}

fn main() {
    let _foreground_hook = win::start_foreground_hook();

    install_panic_log(".".into());

    dioxus::LaunchBuilder::desktop()
        .with_cfg(
            Config::new()
                .with_window(
                    WindowBuilder::new()
                        .with_title("Locked In")
                        .with_resizable(true)
                        .with_visible(false),
                )
                .with_menu(None)
                .with_close_behaviour(dioxus::desktop::WindowCloseBehaviour::WindowHides)
                .with_data_directory("."),
        )
        .launch(App);
}

#[component]
fn App() -> Element {
    let icon = Icon::from_path(dioxus::asset_resolver::asset_path(FAVICON).unwrap(), None).unwrap();

    let menu = Menu::new();
    let menu_item_quit = MenuItem::with_id("quit", "Quit", true, None);
    let menu_item_catpure_focused_window = MenuItem::with_id(
        "capture_focused_window",
        "Capture Focused Window (F3)",
        true,
        None,
    );
    let menu_item_toggle = MenuItem::with_id("toggle", "Toggle", true, None);

    menu.append_items(&[
        &menu_item_quit,
        &menu_item_catpure_focused_window,
        &menu_item_toggle,
    ])
    .unwrap();

    let builder = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_icon(icon);

    provide_context(builder.build().expect("Failed to build tray icon"));

    let mut captured_window: Signal<Option<win::WindowMetadata>> = use_signal(|| None);
    let mut capture_window_shortcut_armed = use_signal(|| false);

    use_muda_event_handler(move |event| match event.id.0.as_str() {
        "quit" => {
            std::process::exit(0);
        }
        "capture_focused_window" => {
            println!("Capture Focused Window");
            capture_window_shortcut_armed.set(true);
            spawn(async move {
                let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(15));
                interval.tick().await;
                interval.tick().await;
                if capture_window_shortcut_armed() {
                    capture_window_shortcut_armed.set(false);
                }
            });
        }
        "toggle" => {
            println!("Toggle clicked");
        }
        _ => {
            println!("To menu item found");
        }
    });

    use_future(move || async move {
        let mut rx = win::FOCUSED_WINDOW_TX.subscribe();
        loop {
            if rx.changed().await.is_err() {
                break;
            }
            let focused_window = rx.borrow().clone();
            *FOCUSED_WINDOW_SIGNAL.write() = focused_window.clone();

            let config = CONFIG_SIGNAL.read();
            for rule in config.rules.iter() {
                if let config::Event::FocusedWindowChanged(_) = rule.event {
                    rule.trigger(&focused_window);
                }
            }
        }
    });

    rsx!(
        Main {},
        if capture_window_shortcut_armed() {
            CaptureFocusedWindowShortcut {
                captured_window: captured_window,
                armed: capture_window_shortcut_armed,
            }
        }
        if let Some(_) = captured_window() {
            Dialog {
                title: "Captured Window".to_string(),
                hide_buttons: true,
                on_cancel: move |_| captured_window.set(None),
                CaptureFocusedWindow {
                    captured_window: captured_window,
                    on_submit: move |_| {
                        captured_window.set(None);
                    }
                }
            }
        }
    )
}

#[component]
fn Main() -> Element {
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

    let mut show_edit_rule_modal = use_signal(|| false);

    let mut rule_to_edit: Signal<Option<String>> = use_signal(|| None);

    rsx! {
        document::Link { rel: "icon", href: FAVICON }
        document::Link { rel: "stylesheet", href: MAIN_CSS }
        main {
            class: "container",
            // h2 { "Window data: {focused_window_title} - {focused_window_class}" }
            // div {
            //     button {
            //         onclick: move |_| {
            //             let mut config = CONFIG_SIGNAL.write();
            //             config.rules.push(config::Rule::default());
            //         },
            //         "Add rule"
            //     }
            //     button {
            //         onclick: move |_| {
            //             let _ = CONFIG_SIGNAL.read().save();
            //         },
            //         "Save Config"
            //     }
            // }
            Rules {
                selected_rule: rule_to_edit,
            },
            if let Some(rule_to_edit) = rule_to_edit() && show_edit_rule_modal() {
                Dialog {
                    title: "Rule".to_string(),
                    hide_buttons: true,
                    on_cancel: move |_| show_edit_rule_modal.set(false),
                    EditRule {
                        rule_name: rule_to_edit,
                        on_submit: move |_| show_edit_rule_modal.set(false),
                    }
                },
            }
        }
    }
}
