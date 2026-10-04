use dioxus::prelude::*;

use super::{armed_capture_shortcut::ArmedCaptureShortcut, capture};

#[component]
pub(super) fn CaptureShortcut() -> Element {
    let session = capture::session();
    if !session.is_armed() {
        return rsx! {};
    }
    let generation = session.generation();
    rsx! { ArmedCaptureShortcut { key: "{generation}", generation } }
}
