use std::ops::Deref;

use dioxus::prelude::*;

use crate::{
    components::{
        dialog::Dialog,
        events::{edit_window::EditWindow, event_configurator::EventConfiguratorProps},
    },
    config, win,
};

#[derive(Debug, Copy, Clone)]
enum WindowListKind {
    Inclusion,
    Exclusion,
}

#[derive(Debug, Copy, Clone)]
struct EditTarget {
    kind: WindowListKind,
    index: Option<usize>, // None = "Add", Some(i) = "Edit existing i"
}

#[component]
pub fn FocusedWindowChanged(props: EventConfiguratorProps) -> Element {
    let mut show_window_editor = use_signal(|| false);
    let mut draft_window = use_signal(win::WindowMetadata::default);
    let mut edit_target = use_signal(|| None::<EditTarget>);

    let event_read = props.event.read();
    let config::Event::FocusedWindowChanged(event_cfg) = event_read.deref() else {
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
                                onclick: move |_| {
                                    if let config::Event::FocusedWindowChanged(event_cfg) = &*props.event.read()
                                        && let Some(window) = event_cfg.inclusions.get(i).cloned()
                                    {
                                        draft_window.set(window);
                                    }
                                    edit_target.set(Some(EditTarget {
                                        kind: WindowListKind::Inclusion,
                                        index: Some(i),
                                    }));
                                    show_window_editor.set(true);
                                },
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
                    onclick: move |_| {
                        draft_window.set(win::WindowMetadata::default());
                        edit_target.set(Some(EditTarget {
                            kind: WindowListKind::Inclusion,
                            index: None,
                        }));
                        show_window_editor.set(true);
                    },
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
                                onclick: move |_| {
                                    if let config::Event::FocusedWindowChanged(event_cfg) = &*props.event.read()
                                        && let Some(window) = event_cfg.exclusions.get(i).cloned()
                                    {
                                        draft_window.set(window);
                                    }
                                    edit_target.set(Some(EditTarget {
                                        kind: WindowListKind::Exclusion,
                                        index: Some(i),
                                    }));
                                    show_window_editor.set(true);
                                },
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
                    onclick: move |_| {
                        draft_window.set(win::WindowMetadata::default());
                        edit_target.set(Some(EditTarget {
                            kind: WindowListKind::Exclusion,
                            index: None,
                        }));
                        show_window_editor.set(true);
                    },
                    "Add"
                }
            }
        }
        if show_window_editor() {
            Dialog {
                title: "Window".to_string(),
                hide_buttons: true,
                on_cancel: move |_| show_window_editor.set(false),
                EditWindow {
                    window: draft_window,
                    on_submit: {
                        let mut event_signal = props.event;
                        move || {
                            let Some(target) = *edit_target.read() else {
                                show_window_editor.set(false);
                                return;
                            };
                            let new_window = std::mem::take(&mut *draft_window.write());
                            let mut event = event_signal.write();
                            if let config::Event::FocusedWindowChanged(event_cfg) = &mut *event {
                                match (target.kind, target.index) {
                                    (WindowListKind::Inclusion, Some(i)) => {
                                        if i < event_cfg.inclusions.len() {
                                            event_cfg.inclusions[i] = new_window;
                                        }
                                    },
                                    (WindowListKind::Inclusion, None) => {
                                        event_cfg.inclusions.push(new_window);
                                    },
                                    (WindowListKind::Exclusion, Some(i)) => {
                                        if i < event_cfg.exclusions.len() {
                                            event_cfg.exclusions[i] = new_window;
                                        }
                                    },
                                    (WindowListKind::Exclusion, None) => {
                                        event_cfg.exclusions.push(new_window);
                                    }
                                }
                            }
                            edit_target.set(None);
                            show_window_editor.set(false);
                        }
                    }
                }
            }
        }
    )
}
