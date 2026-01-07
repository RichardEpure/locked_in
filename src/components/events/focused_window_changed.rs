use std::ops::Deref;

use dioxus::prelude::*;

use crate::{components::events::event_configurator::EventConfiguratorProps, config};

#[component]
pub fn FocusedWindowChanged(props: EventConfiguratorProps) -> Element {
    let event = props.event.read().deref().clone();
    let config::Event::FocusedWindowChanged(event_cfg) = event else {
        panic!("Expected FocusedWindowChanged");
    };

    rsx!(
        div {
            class: "event-config--focused-window-changed grid",
            div {
                class: "inclusions",
                h6 { "Inclusions" },
                for (i, window) in event_cfg.inclusions.iter().enumerate() {
                    details {
                        class: "window",
                        if let Some(title) = &window.title {
                            summary { "{title}" },
                        } else {
                            summary { "Untitled" },
                        }
                        ul {
                            if let Some(class) = &window.class {
                                li { "{class}" },
                            }
                            if let Some(exe) = &window.exe {
                                li { "{exe.to_string_lossy().to_string()}" },
                            }
                        },
                        div {
                            role: "group",
                            class: "window__buttons",
                            button {
                                class: "outline",
                                "Edit"
                            }
                            button {
                                class: "danger",
                                onclick: {
                                    let mut event_signal = props.event;
                                    move |_| {
                                        let mut event = event_signal.write();
                                        if let config::Event::FocusedWindowChanged(event_cfg) = &mut *event
                                            && i < event_cfg.inclusions.len()
                                        {
                                                event_cfg.inclusions.remove(i);
                                        }
                                    }
                                },
                                "Delete"
                            }
                        }
                    },
                },
                button {
                    class: "outline",
                    "Add"
                }
            },
            div {
                class: "exclusions",
                h6 { "Exclusions" },
                for (i, window) in event_cfg.exclusions.iter().enumerate() {
                    details {
                        class: "window",
                        if let Some(title) = &window.title {
                            summary { "{title}" },
                        } else {
                            summary { "Untitled" },
                        }
                        ul {
                            if let Some(class) = &window.class {
                                li { "{class}" },
                            }
                            if let Some(exe) = &window.exe {
                                li { "{exe.to_string_lossy().to_string()}" },
                            }
                        },
                        div {
                            role: "group",
                            class: "window__buttons",
                            button {
                                class: "outline",
                                "Edit"
                            }
                            button {
                                class: "danger",
                                onclick: {
                                    let mut event_signal = props.event;
                                    move |_| {
                                        let mut event = event_signal.write();
                                        if let config::Event::FocusedWindowChanged(event_cfg) = &mut *event
                                            && i < event_cfg.exclusions.len()
                                        {
                                                event_cfg.exclusions.remove(i);
                                        }
                                    }
                                },
                                "Delete"
                            }
                        }
                    },
                },
                button {
                    class: "outline",
                    "Add"
                }
            }
        }
    )
}
