use dioxus::prelude::*;
use strum::IntoEnumIterator;

use crate::{
    CONFIG_SIGNAL,
    config::{self},
};

#[derive(Props, PartialEq, Clone)]
pub struct EditRuleProps {
    pub rule_name: Option<String>,
    pub on_submit: EventHandler<()>,
}

#[component]
pub fn EditRule(props: EditRuleProps) -> Element {
    let mut rule = use_signal(|| {
        if let Some(rule_name) = props.rule_name.as_ref()
            && let Some(existing_rule) = CONFIG_SIGNAL.read().get_rule(rule_name)
        {
            existing_rule.clone()
        } else {
            config::Rule::default()
        }
    });

    rsx! {
        form {
            class: "edit-rule",
            fieldset {
                label {
                    "Name"
                    input {
                        name: "name",
                        placeholder: "rule name",
                        value: "{rule().name}",
                        oninput: move |e| rule.write().name = e.value()
                    }
                }
                label {
                    "Event",
                    select {
                        name: "event",
                        aria_label: "Select an event",
                        for event in config::Event::iter().map(|event| event.to_string()) {
                            option {
                                selected: if event == rule().event.to_string() { true } else { false },
                                "{event}"
                            }
                        }
                    }
                }
                label {
                    "Devices",
                }
            }
            input {
                type: "submit",
                onclick: move |_| {
                    let mut config = CONFIG_SIGNAL.write();
                    if let Some(rule_name) = props.rule_name.as_ref()
                        && let Some(index) = config.get_rule_index(rule_name)
                    {
                        config.rules[index] = rule().clone();
                    }
                    props.on_submit.call(());
                },
                "Submit",
            }
        }
    }
}
