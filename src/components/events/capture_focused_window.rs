use std::path::PathBuf;

use dioxus::prelude::*;

use crate::{CONFIG_SIGNAL, config, win};

#[derive(Props, PartialEq, Clone)]
pub struct CaptureFocusedWindowProps {
    pub captured_window: Signal<Option<win::WindowMetadata>>,
    pub on_submit: EventHandler<()>,
}

#[component]
pub fn CaptureFocusedWindow(props: CaptureFocusedWindowProps) -> Element {
    let mut captured_window = props.captured_window;
    let Some(window) = &*captured_window.read() else {
        panic!("Expected some captured_window value");
    };

    let mut window = use_signal(|| window.clone());

    let config = CONFIG_SIGNAL.read();

    let rules = config
        .rules
        .iter()
        .filter(|r| matches!(r.event, config::Event::FocusedWindowChanged(_)));

    let mut selected_rule_name = use_signal(String::default);

    // Whether to add the captured window to inclusions or exclusions.
    let mut inclusions = use_signal(|| true);

    let (title, class, exe) = {
        let w = window.read();
        (
            w.title.clone().unwrap_or_default(),
            w.class.clone().unwrap_or_default(),
            w.exe
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
        )
    };

    rsx!(
        form {
            label {
                "Rule"
                select {
                    name: "rule",
                    aria_label: "Select a rule",
                    oninput: move |e| {
                        selected_rule_name.set(e.value());
                    },
                    option {
                        selected: true,
                        disabled: true,
                        "Select a rule"
                    }
                    for rule in rules {
                        option {
                            "{rule.name}"
                        }
                    }
                }
            }
            fieldset {
                legend { "Event Config" }
                input {
                    type: "radio",
                    id: "inclusions",
                    name: "event_config",
                    checked: inclusions(),
                    onchange: move |_| {
                        inclusions.set(true);
                    }
                }
                label {
                    "htmlFor": "inclusions",
                    "Inclusions"
                }
                input {
                    type: "radio",
                    id: "exclusions",
                    name: "event_config",
                    checked: !inclusions(),
                    onchange: move |_| {
                        inclusions.set(false);
                    }
                }
                label {
                    "htmlFor": "exclusions",
                    "Exclusions"
                }
            }
            fieldset {
                label {
                    "Title",
                    input {
                        name: "title",
                        placeholder: "title",
                        value: "{title}",
                        oninput: move |e| {
                            let value = e.value().trim().to_string();
                            window.write().title = if value.is_empty() { None } else { Some(value) };
                        }
                    }
                },
                label {
                    "Class",
                    input {
                        name: "class",
                        placeholder: "class",
                        value: "{class}",
                        oninput: move |e| {
                            let value = e.value().trim().to_string();
                            window.write().class = if value.is_empty() { None } else { Some(value) };
                        }
                    }
                }
                label {
                    "Exe Path",
                    input {
                        name: "exe",
                        placeholder: r"C:\Path\To\App.exe",
                        value: "{exe}",
                        oninput: move |e| {
                            let value = e.value().trim().to_string();
                            window.write().exe = if value.is_empty() {
                                None
                            } else {
                                Some(PathBuf::from(value))
                            };
                        }
                    }
                }
            }
            input {
                type: "submit",
                onclick: move |_| {
                    let window = std::mem::take(&mut *window.write());
                    let mut config = CONFIG_SIGNAL.write();
                    let rule = config.get_mut_rule(&selected_rule_name());
                    if let Some(rule) = rule
                        && let config::Event::FocusedWindowChanged(event_cfg) = &mut rule.event {
                        if inclusions() {
                            event_cfg.inclusions.push(window);
                        } else {
                            event_cfg.exclusions.push(window);
                        }
                    }
                    props.on_submit.call(());
                },
                "Submit",
            }
        }
    )
}
