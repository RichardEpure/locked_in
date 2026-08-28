use std::collections::HashSet;

use dioxus::prelude::*;

use crate::{
    CAPTURE_ARMED_SIGNAL, CAPTURE_TARGET_SIGNAL, CaptureTarget, arm_capture, cancel_capture,
    config::{Automation, WindowMatcher},
};

use super::{
    matcher_editor::MatcherEditor,
    mutations::{add_matcher, matcher_group_body_id, reveal_last_matcher},
};

#[derive(Props, Clone, PartialEq)]
pub(super) struct MatcherGroupProps {
    draft: Signal<Automation>,
    collapsed_matcher_groups: Signal<HashSet<(String, bool)>>,
    case_index: usize,
    case_id: String,
    case_name: String,
    exceptions: bool,
    matchers: Vec<WindowMatcher>,
}

#[component]
pub(super) fn MatcherGroup(props: MatcherGroupProps) -> Element {
    let mut draft = props.draft;
    let mut collapsed_matcher_groups = props.collapsed_matcher_groups;
    let case_index = props.case_index;
    let exceptions = props.exceptions;
    let title = if exceptions {
        "Except when"
    } else {
        "Applications"
    };
    let copy = if exceptions {
        "Any match here skips this case"
    } else {
        "Any application may match; populated fields are ANDed"
    };
    let group_key = (props.case_id.clone(), exceptions);
    let collapsed = collapsed_matcher_groups.read().contains(&group_key);
    let body_id = matcher_group_body_id(case_index, exceptions);
    let capture_case_id = props.case_id.clone();
    let add_case_id = props.case_id.clone();
    let capture_target = CAPTURE_TARGET_SIGNAL.read().clone();
    let capture_armed = *CAPTURE_ARMED_SIGNAL.read()
        && capture_target.as_ref().is_some_and(|target| {
            let automation = draft.read();
            target.automation_id == automation.id
                && target.case_id == props.case_id
                && target.exception == exceptions
        });
    rsx! {
        div { class: if exceptions { if collapsed { "matcher-group exceptions collapsed" } else { "matcher-group exceptions" } } else if collapsed { "matcher-group collapsed" } else { "matcher-group" },
            div { class: "matcher-group__heading",
                div { class: "matcher-group__summary",
                    button {
                        class: "matcher-group__toggle",
                        aria_expanded: !collapsed,
                        aria_controls: "{body_id}",
                        onclick: move |_| {
                            let mut groups = collapsed_matcher_groups.write();
                            if !groups.remove(&group_key) {
                                groups.insert(group_key.clone());
                            }
                        },
                        span { class: "disclosure-icon", aria_hidden: true, if collapsed { ">" } else { "v" } }
                        span { "{title}" }
                        span { class: "matcher-count", "{props.matchers.len()}" }
                    }
                    p { "{copy}" }
                }
                div { class: "toolbar tight",
                    button { class: "button ghost small", onclick: move |_| {
                        collapsed_matcher_groups.write().remove(&(capture_case_id.clone(), exceptions));
                        let automation = draft.read();
                        arm_capture(Some(CaptureTarget::new(automation.id.clone(), capture_case_id.clone(), exceptions)));
                    }, if capture_armed { "Waiting for F3" } else { "Capture next (F3)" } }
                    button { class: "button secondary small", onclick: move |_| {
                        collapsed_matcher_groups.write().remove(&(add_case_id.clone(), exceptions));
                        add_matcher(&mut draft, case_index, exceptions);
                        reveal_last_matcher(case_index, exceptions);
                    }, "+ Add matcher" }
                }
            }
            if capture_armed {
                div { class: "capture-status", role: "status", aria_live: "polite",
                    span { "Capturing for \"{props.case_name}\" -> {title}. Focus another window, then press F3." }
                    button { class: "button ghost small", onclick: move |_| cancel_capture(), "Cancel" }
                }
            }
            div { class: "matcher-group__body", id: "{body_id}", hidden: collapsed,
                for (matcher_index, matcher) in props.matchers.iter().cloned().enumerate() {
                    MatcherEditor { key: "{matcher.id}", draft, case_index, exceptions, matcher_index, matcher }
                }
                if props.matchers.is_empty() && exceptions { div { class: "inline-empty compact", "No exceptions." } }
            }
        }
    }
}
