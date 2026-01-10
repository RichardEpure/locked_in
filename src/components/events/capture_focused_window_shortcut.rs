use dioxus::{
    desktop::{HotKeyState, use_global_shortcut, use_window},
    prelude::*,
};

use crate::{FOCUSED_WINDOW_SIGNAL, win};

#[derive(Props, PartialEq, Clone)]
pub struct CaptureFocusedWindowShortcutProps {
    pub captured_window: Signal<Option<win::WindowMetadata>>,
    pub armed: Signal<bool>,
}

// This component is used just to create a global shortcut that only lives for
// as long as the component is alive.
#[component]
pub fn CaptureFocusedWindowShortcut(props: CaptureFocusedWindowShortcutProps) -> Element {
    let mut captured_window = props.captured_window;
    let mut armed = props.armed;
    let app_window = use_window();

    let _ = use_global_shortcut(KeyCode::F3, move |state| {
        if !armed() || state != HotKeyState::Pressed {
            return;
        }

        captured_window.set(Some(FOCUSED_WINDOW_SIGNAL.read().clone()));
        println!("{:?}", captured_window());

        app_window.set_visible(true);
        app_window.set_minimized(false);
        app_window.set_focus();

        armed.set(false);
    });
    rsx!()
}
