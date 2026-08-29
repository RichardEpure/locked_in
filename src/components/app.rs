use std::sync::Arc;

use dioxus::{
    desktop::{
        WindowCloseBehaviour,
        trayicon::{
            Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent,
            menu::{Menu, MenuItem},
        },
        use_muda_event_handler, use_tray_icon_event_handler, use_window,
    },
    prelude::*,
};

use crate::{
    FOCUSED_WINDOW_SIGNAL, app_log, arm_capture,
    automation_runtime::AutomationRuntime,
    config::{LogLevel, PublishedConfig},
    focused_window::FocusedWindow,
    win,
};

use super::{PublishedConfigContext, capture_shortcut::CaptureShortcut, workspace::Workspace};

const FAVICON: Asset = asset!("/assets/favicon.ico");
const MAIN_CSS: Asset = asset!("/assets/styles/main.css");
const ROBOTO_FONT: Asset = asset!(
    "/assets/fonts/Roboto-VariableFont_wdth,wght.ttf",
    AssetOptions::builder().with_hash_suffix(false)
);

#[component]
pub(crate) fn App() -> Element {
    let window = use_window();
    let runtime = consume_context::<AutomationRuntime>();
    let publication_subscription =
        consume_context::<Option<tokio::sync::watch::Receiver<Arc<PublishedConfig>>>>();
    let publication = use_signal(|| None::<Arc<PublishedConfig>>);
    use_context_provider(move || PublishedConfigContext::new(publication));
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
    let tray_available = if menu_ready {
        match builder.build() {
            Ok(tray) => {
                provide_context(tray);
                true
            }
            Err(_) => false,
        }
    } else {
        false
    };
    if !tray_available {
        app_log::write("tray icon could not be created");
        window.set_visible(true);
    }
    use_future({
        let window = window.clone();
        move || {
            let mut subscription = publication_subscription.clone();
            let window = window.clone();
            async move {
                let Some(subscription) = subscription.as_mut() else {
                    return;
                };
                project_current_publication(subscription, tray_available, |projection| {
                    apply_publication(projection, &window, publication);
                });
                while subscription.changed().await.is_ok() {
                    project_current_publication(subscription, tray_available, |projection| {
                        apply_publication(projection, &window, publication);
                    });
                }
            }
        }
    });

    use_muda_event_handler({
        let window = window.clone();
        let runtime = runtime.clone();
        move |event| match event.id.0.as_str() {
            "quit" => {
                runtime.request_shutdown();
                window.set_close_behavior(WindowCloseBehaviour::WindowCloses);
                window.close();
            }
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
        let mut receiver = win::subscribe_focused_window();
        publish_current_focused_window(&mut receiver, |focused| {
            *FOCUSED_WINDOW_SIGNAL.write() = focused;
        });
        while receiver.changed().await.is_ok() {
            *FOCUSED_WINDOW_SIGNAL.write() = receiver.borrow_and_update().clone();
        }
    });

    rsx! {
        document::Link { rel: "icon", href: FAVICON }
        document::Link { rel: "preload", href: ROBOTO_FONT, as: "font", r#type: "font/ttf", crossorigin: "anonymous" }
        document::Link { rel: "stylesheet", href: MAIN_CSS }
        CaptureShortcut {}
        Workspace {}
    }
}

struct PublicationProjection {
    publication: Arc<PublishedConfig>,
    log_level: LogLevel,
    close_behavior: WindowCloseBehaviour,
}

fn project_current_publication(
    receiver: &mut tokio::sync::watch::Receiver<Arc<PublishedConfig>>,
    tray_available: bool,
    publish: impl FnOnce(PublicationProjection),
) {
    let publication = receiver.borrow_and_update().clone();
    let projection = PublicationProjection {
        log_level: publication.editable().settings.log_level,
        close_behavior: effective_close_behavior(
            publication.editable().settings.close_to_tray,
            tray_available,
        ),
        publication,
    };
    publish(projection);
}

fn apply_publication(
    projection: PublicationProjection,
    window: &dioxus::desktop::DesktopContext,
    mut publication: Signal<Option<Arc<PublishedConfig>>>,
) {
    app_log::set_level(projection.log_level);
    window.set_close_behavior(projection.close_behavior);
    publication.set(Some(projection.publication));
}

fn effective_close_behavior(close_to_tray: bool, tray_available: bool) -> WindowCloseBehaviour {
    if close_to_tray && tray_available {
        WindowCloseBehaviour::WindowHides
    } else {
        WindowCloseBehaviour::WindowCloses
    }
}

fn publish_current_focused_window(
    receiver: &mut tokio::sync::watch::Receiver<FocusedWindow>,
    publish: impl FnOnce(FocusedWindow),
) {
    publish(receiver.borrow_and_update().clone());
}

#[cfg(test)]
mod tests;
