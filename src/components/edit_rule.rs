use dioxus::prelude::*;
use strum::IntoEnumIterator;

use crate::{
    CONFIG_SIGNAL,
    components::events::event_configurator::EventConfigurator,
    config::{self},
};

#[derive(Props, PartialEq, Clone)]
pub struct EditRuleProps {
    pub rule_name: String,
    pub on_submit: EventHandler<()>,
}

#[component]
pub fn EditRule(props: EditRuleProps) -> Element {
    let mut rule = use_signal(|| {
        if let Some(existing_rule) = CONFIG_SIGNAL.read().get_rule(&props.rule_name) {
            existing_rule.clone()
        } else {
            config::Rule::default()
        }
    });

    let event_signal = use_signal(|| rule().event);

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
                        oninput: move |e| {
                            if let Ok(event) = e.value().parse::<config::Event>() {
                                rule.write().event = event;
                            }
                        },
                        for event in config::Event::iter().map(|event| event.to_string()) {
                            option {
                                selected: if event == rule().event.to_string() { true } else { false },
                                "{event}"
                            }
                        }
                    },
                    EventConfigurator {
                        event: event_signal,
                    }
                },
                label {
                    "Devices",
                }
            }
            input {
                type: "submit",
                onclick: move |_| {
                    let mut config = CONFIG_SIGNAL.write();
                    rule.write().event = event_signal.read().clone();
                    if let Some(index) = config.get_rule_index(&props.rule_name)
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
