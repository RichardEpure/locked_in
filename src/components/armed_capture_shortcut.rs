use dioxus::{
    desktop::{HotKeyState, use_global_shortcut, use_window},
    prelude::*,
};

use crate::{
    CAPTURE_ARMED_SIGNAL, CAPTURE_GENERATION_SIGNAL, CAPTURED_WINDOW_SIGNAL, FOCUSED_WINDOW_SIGNAL,
    app_log,
};

#[component]
pub(super) fn ArmedCaptureShortcut() -> Element {
    let window = use_window();
    let _shortcut = use_global_shortcut(KeyCode::F3, move |state| {
        if state != HotKeyState::Pressed {
            return;
        }
        *CAPTURE_GENERATION_SIGNAL.write() += 1;
        *CAPTURED_WINDOW_SIGNAL.write() = Some(FOCUSED_WINDOW_SIGNAL.read().clone());
        *CAPTURE_ARMED_SIGNAL.write() = false;
        window.set_visible(true);
        window.set_minimized(false);
        window.set_focus();
        app_log::write("focused window captured");
    });
    rsx! {}
}
