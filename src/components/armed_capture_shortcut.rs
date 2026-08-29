use dioxus::{
    desktop::{HotKeyState, use_global_shortcut, use_window},
    prelude::*,
};

use crate::{
    CAPTURE_ARMED_SIGNAL, CAPTURE_GENERATION_SIGNAL, CAPTURE_TARGET_SIGNAL, CAPTURED_WINDOW_SIGNAL,
    CaptureTarget, CapturedWindow, FOCUSED_WINDOW_SIGNAL, app_log,
};

#[derive(Props, Clone, PartialEq)]
pub(super) struct ArmedCaptureShortcutProps {
    generation: u64,
    target: Option<CaptureTarget>,
}

#[component]
pub(super) fn ArmedCaptureShortcut(props: ArmedCaptureShortcutProps) -> Element {
    let window = use_window();
    let _shortcut = use_global_shortcut(KeyCode::F3, move |state| {
        if state != HotKeyState::Pressed {
            return;
        }
        let current_generation = *CAPTURE_GENERATION_SIGNAL.read();
        let current_target = CAPTURE_TARGET_SIGNAL.read().clone();
        if !capture_session_is_current(
            props.generation,
            &props.target,
            current_generation,
            &current_target,
            *CAPTURE_ARMED_SIGNAL.read(),
        ) {
            return;
        }
        *CAPTURED_WINDOW_SIGNAL.write() = Some(CapturedWindow {
            generation: props.generation,
            target: props.target.clone(),
            window: FOCUSED_WINDOW_SIGNAL.read().clone(),
        });
        *CAPTURE_ARMED_SIGNAL.write() = false;
        window.set_visible(true);
        window.set_minimized(false);
        window.set_focus();
        app_log::write("focused window captured");
    });
    rsx! {}
}

fn capture_session_is_current(
    callback_generation: u64,
    callback_target: &Option<CaptureTarget>,
    current_generation: u64,
    current_target: &Option<CaptureTarget>,
    armed: bool,
) -> bool {
    armed && callback_generation == current_generation && callback_target == current_target
}

#[cfg(test)]
mod tests;
