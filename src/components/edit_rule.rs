use std::ops::Deref;

use dioxus::prelude::*;

use crate::{CONFIG_SIGNAL, config::Rule};

#[derive(Props, PartialEq, Clone)]
pub struct EditRuleProps {
    pub rule_name: Option<String>,
}

#[component]
pub fn EditRule(props: EditRuleProps) -> Element {
    let mut rule: Option<Rule> = props.rule_name.map(|rule_name| {
        CONFIG_SIGNAL
            .read()
            .deref()
            .rules
            .iter()
            .find(|r| r.name == rule_name)
            .unwrap_or_else(|| panic!("Trying to edit a rule {} that doesn't exist.", rule_name))
            .clone()
    });

    let name = if rule.is_some() {
        rule.unwrap().name.clone()
    } else {
        "test".to_string()
    };

    rsx! {
        div {
            class: "edit-rule",
            "{name}"
        }
    }
}
