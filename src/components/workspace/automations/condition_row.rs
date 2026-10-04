use dioxus::prelude::*;
use dioxus_icons::lucide::CaseSensitive;

use super::draft::AutomationDraft;
use crate::config::{MatchOperator, TextCondition, WindowMatcher};

#[derive(Props, Clone, PartialEq)]
pub(super) struct ConditionRowProps {
    draft: Signal<AutomationDraft>,
    case_index: usize,
    exceptions: bool,
    matcher_index: usize,
    field: MatcherField,
    condition: Option<TextCondition>,
}

#[component]
pub(super) fn ConditionRow(props: ConditionRowProps) -> Element {
    let mut draft = props.draft;
    let value = props
        .condition
        .as_ref()
        .map(|condition| condition.value.clone())
        .unwrap_or_default();
    let operator = props
        .condition
        .as_ref()
        .map_or(props.field.default_operator(), |condition| {
            condition.operator
        });
    let case_sensitive = props
        .condition
        .as_ref()
        .is_some_and(|condition| condition.case_sensitive);
    let field = props.field;
    let label = field.label();
    let mut update = move |edit| {
        let case = &mut draft.write().edited.cases[props.case_index];
        let matcher = if props.exceptions {
            &mut case.exceptions[props.matcher_index]
        } else {
            &mut case.applications[props.matcher_index]
        };
        field.apply(matcher, edit);
    };
    rsx! {
        div { class: "condition-row",
            span { class: "condition-label", "{label}" }
            select { value: operator_name(operator), onchange: move |event| update(ConditionEdit::Operator(parse_operator(&event.value()))),
                option { value: "contains", "contains" }
                option { value: "equals", "equals" }
                option { value: "regex", "regex" }
            }
            input { placeholder: "Not used", value: "{value}", oninput: move |event| update(ConditionEdit::Value(event.value())) }
            label { class: "case-check", title: "Case sensitive", input { type: "checkbox", aria_label: "Case sensitive", checked: case_sensitive, onchange: move |event| update(ConditionEdit::CaseSensitive(event.checked())) } CaseSensitive { size: 16, "aria-hidden": "true" } }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum MatcherField {
    Title,
    Class,
    Executable,
}

enum ConditionEdit {
    Operator(MatchOperator),
    Value(String),
    CaseSensitive(bool),
}

impl MatcherField {
    fn label(self) -> &'static str {
        match self {
            Self::Title => "Window title",
            Self::Class => "Window class",
            Self::Executable => "Executable",
        }
    }

    fn default_operator(self) -> MatchOperator {
        match self {
            Self::Title | Self::Class => MatchOperator::Contains,
            Self::Executable => MatchOperator::Equals,
        }
    }

    fn slot(self, matcher: &mut WindowMatcher) -> &mut Option<TextCondition> {
        match self {
            Self::Title => &mut matcher.title,
            Self::Class => &mut matcher.class,
            Self::Executable => &mut matcher.exe,
        }
    }

    fn apply(self, matcher: &mut WindowMatcher, edit: ConditionEdit) {
        let slot = self.slot(matcher);
        let mut condition = slot.take().unwrap_or(TextCondition {
            operator: self.default_operator(),
            value: String::new(),
            case_sensitive: false,
        });
        match edit {
            ConditionEdit::Operator(value) => condition.operator = value,
            ConditionEdit::Value(value) => condition.value = value,
            ConditionEdit::CaseSensitive(value) => condition.case_sensitive = value,
        }
        *slot = (!condition.value.is_empty()).then_some(condition);
    }
}

fn operator_name(operator: MatchOperator) -> &'static str {
    match operator {
        MatchOperator::Equals => "equals",
        MatchOperator::Contains => "contains",
        MatchOperator::Regex => "regex",
    }
}

fn parse_operator(value: &str) -> MatchOperator {
    match value {
        "equals" => MatchOperator::Equals,
        "regex" => MatchOperator::Regex,
        _ => MatchOperator::Contains,
    }
}

#[cfg(test)]
mod tests;
