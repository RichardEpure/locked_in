use dioxus::prelude::*;
use dioxus_icons::lucide::Trash2;

use crate::config::WindowMatcher;

use super::{
    condition_row::{ConditionRow, MatcherField},
    draft::AutomationDraft,
};

#[derive(Props, Clone, PartialEq)]
pub(super) struct MatcherEditorProps {
    draft: Signal<AutomationDraft>,
    case_index: usize,
    exceptions: bool,
    matcher_index: usize,
    matcher: WindowMatcher,
}

#[component]
pub(super) fn MatcherEditor(props: MatcherEditorProps) -> Element {
    let mut draft = props.draft;
    let case_index = props.case_index;
    let exceptions = props.exceptions;
    let matcher_index = props.matcher_index;
    let matcher = props.matcher;
    rsx! {
        div { class: "matcher-card",
            ConditionRow { draft, case_index, exceptions, matcher_index, field: MatcherField::Title, condition: matcher.title.clone() }
            ConditionRow { draft, case_index, exceptions, matcher_index, field: MatcherField::Class, condition: matcher.class.clone() }
            ConditionRow { draft, case_index, exceptions, matcher_index, field: MatcherField::Executable, condition: matcher.exe.clone() }
            button { class: "icon-button danger matcher-remove", aria_label: "Remove matcher", title: "Remove matcher", onclick: move |_| {
                let case = &mut draft.write().edited.cases[case_index];
                if exceptions { case.exceptions.remove(matcher_index); } else { case.applications.remove(matcher_index); }
            }, Trash2 { size: 16, "aria-hidden": "true" } }
        }
    }
}
