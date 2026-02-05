use dioxus::prelude::*;

use crate::CONFIG_SIGNAL;

#[derive(Props, PartialEq, Clone)]
pub struct RulesProps {
    pub selected_rule: Signal<Option<String>>,
}

#[component]
pub fn Rules(props: RulesProps) -> Element {
    let mut selected = props.selected_rule;
    let rule_names: Vec<String> = CONFIG_SIGNAL
        .read()
        .rules
        .iter()
        .map(|r| r.name.clone())
        .collect();

    rsx! {
        div {
            class: "rules",
            h2 {
                class: "rules__title",
                "Rules"
            }
            ul {
                class: "rules__list",
                for name in rule_names {
                    li {
                        class: "rules__item",
                        class: if selected().as_ref() == Some(&name) { "selected" } else { "" },
                        onclick: move |_| {
                            selected.set(Some(name.clone()));
                        },
                        "{name}"
                    }
                }
            }
        }
    }
}
