use dioxus::{
    desktop::{HotKeyState, use_global_shortcut, use_window},
    prelude::*,
};

use super::capture;
use crate::{FOCUSED_WINDOW_SIGNAL, app_log};

#[derive(Props, Clone, PartialEq)]
pub(super) struct ArmedCaptureShortcutProps {
    generation: u64,
}

#[component]
pub(super) fn ArmedCaptureShortcut(props: ArmedCaptureShortcutProps) -> Element {
    let window = use_window();
    let _shortcut = use_global_shortcut(KeyCode::F3, move |state| {
        if state != HotKeyState::Pressed {
            return;
        }
        if !capture::capture(props.generation, FOCUSED_WINDOW_SIGNAL.read().clone()) {
            return;
        }
        window.set_visible(true);
        window.set_minimized(false);
        window.set_focus();
        app_log::write("focused window captured");
    });
    rsx! {}
}
