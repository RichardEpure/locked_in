use dioxus::prelude::*;

use crate::config::{Automation, MatchOperator, TextCondition};

#[derive(Props, Clone, PartialEq)]
pub(super) struct ConditionRowProps {
    draft: Signal<Automation>,
    case_index: usize,
    exceptions: bool,
    matcher_index: usize,
    field: &'static str,
    label: &'static str,
    condition: Option<TextCondition>,
    default_operator: MatchOperator,
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
        .map_or(props.default_operator, |condition| condition.operator);
    let case_sensitive = props
        .condition
        .as_ref()
        .is_some_and(|condition| condition.case_sensitive);
    let operator_props = props.clone();
    let value_props = props.clone();
    let case_props = props.clone();
    rsx! {
        div { class: "condition-row",
            span { class: "condition-label", "{props.label}" }
            select { value: operator_name(operator), onchange: move |event| update_condition(&mut draft, &operator_props, Some(parse_operator(&event.value())), None, None),
                option { value: "contains", "contains" }
                option { value: "equals", "equals" }
                option { value: "regex", "regex" }
            }
            input { placeholder: "Not used", value: "{value}", oninput: move |event| update_condition(&mut draft, &value_props, None, Some(event.value()), None) }
            label { class: "case-check", input { type: "checkbox", checked: case_sensitive, onchange: move |event| update_condition(&mut draft, &case_props, None, None, Some(event.checked())) } "Aa" }
        }
    }
}

fn update_condition(
    draft: &mut Signal<Automation>,
    props: &ConditionRowProps,
    operator: Option<MatchOperator>,
    value: Option<String>,
    case_sensitive: Option<bool>,
) {
    let case = &mut draft.write().cases[props.case_index];
    let matcher = if props.exceptions {
        &mut case.exceptions[props.matcher_index]
    } else {
        &mut case.applications[props.matcher_index]
    };
    let slot = match props.field {
        "title" => &mut matcher.title,
        "class" => &mut matcher.class,
        _ => &mut matcher.exe,
    };
    let mut condition = slot.clone().unwrap_or(TextCondition {
        operator: props.default_operator,
        value: String::new(),
        case_sensitive: false,
    });
    if let Some(operator) = operator {
        condition.operator = operator;
    }
    if let Some(value) = value {
        condition.value = value;
    }
    if let Some(case_sensitive) = case_sensitive {
        condition.case_sensitive = case_sensitive;
    }
    *slot = if condition.value.is_empty() {
        None
    } else {
        Some(condition)
    };
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
