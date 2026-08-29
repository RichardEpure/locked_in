use dioxus::prelude::*;

use crate::{CAPTURE_ARMED_SIGNAL, CAPTURE_GENERATION_SIGNAL, CAPTURE_TARGET_SIGNAL};

use super::armed_capture_shortcut::ArmedCaptureShortcut;

#[component]
pub(super) fn CaptureShortcut() -> Element {
    if !*CAPTURE_ARMED_SIGNAL.read() {
        return rsx! {};
    }
    let generation = *CAPTURE_GENERATION_SIGNAL.read();
    let target = CAPTURE_TARGET_SIGNAL.read().clone();
    rsx! { ArmedCaptureShortcut { key: "{generation}", generation, target } }
}
