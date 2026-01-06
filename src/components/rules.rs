use dioxus::prelude::*;

use crate::CONFIG_SIGNAL;

#[derive(Props, PartialEq, Clone)]
pub struct RulesProps {
    pub on_edit: EventHandler<String>,
}

#[component]
pub fn Rules(props: RulesProps) -> Element {
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
            div {
                class: "rules__list",
                for name in rule_names {
                    article {
                        class: "rules__item",
                        "{name}"
                        div {
                            role: "group",
                            class: "buttons",
                            button {
                                onclick: {
                                    let name = name.clone();
                                    move |_| props.on_edit.call(name.clone())
                                },
                                "Edit"
                            }
                            button {
                                class: "danger",
                                onclick: move |_| {
                                    CONFIG_SIGNAL.write().delete_rule(&name);
                                },
                                "Delete"
                            }
                        }
                    }
                }
            }
        }
    }
}
