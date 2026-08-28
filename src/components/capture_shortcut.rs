use dioxus::prelude::*;

use crate::CAPTURE_ARMED_SIGNAL;

use super::armed_capture_shortcut::ArmedCaptureShortcut;

#[component]
pub(super) fn CaptureShortcut() -> Element {
    if !*CAPTURE_ARMED_SIGNAL.read() {
        return rsx! {};
    }
    rsx! { ArmedCaptureShortcut {} }
}
