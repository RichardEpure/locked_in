use std::ops::{Deref, DerefMut};

use dioxus::prelude::*;

use crate::CONFIG_SIGNAL;

#[component]
pub fn Rules() -> Element {
    let rule_names: Vec<String> = CONFIG_SIGNAL
        .read()
        .deref()
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
                            button { "Edit" }
                            button {
                                class: "danger",
                                onclick: move |_| {
                                    CONFIG_SIGNAL.write().deref_mut().delete_role(&name);
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
