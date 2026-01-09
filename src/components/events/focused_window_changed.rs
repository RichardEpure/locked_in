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

    let mut show_report_editor = use_signal(|| false);
    let mut draft_report = use_signal(Vec::<u8>::default);
    let mut on_match = use_signal(|| true);

    let event_read = props.event.read();
    let config::Event::FocusedWindowChanged(event_cfg) = event_read.deref() else {
        panic!("Expected FocusedWindowChanged");
    };

    rsx!(
        div {
            class: "event-config--focused-window-changed",
            div {
                class: "grid",
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
                                    onclick: move |_| {
                                        let mut event_signal = props.event;
                                        let mut event = event_signal.write();
                                        if let config::Event::FocusedWindowChanged(event_cfg) = &mut *event
                                            && i < event_cfg.inclusions.len()
                                        {
                                                event_cfg.inclusions.remove(i);
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
                                    onclick: move |_| {
                                        let mut event_signal = props.event;
                                        let mut event = event_signal.write();
                                        if let config::Event::FocusedWindowChanged(event_cfg) = &mut *event
                                            && i < event_cfg.exclusions.len()
                                        {
                                                event_cfg.exclusions.remove(i);
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
            hr {}
            div {
                class: "grid",
                div {
                    h6 { "On Match Reports" },
                    for (i, report) in event_cfg.on_match_reports.iter().enumerate() {
                        details {
                            summary { "{hex::encode(report)}" }
                            button {
                                class: "danger",
                                onclick: move |_| {
                                    let mut event_signal = props.event;
                                    let mut event = event_signal.write();
                                    if let config::Event::FocusedWindowChanged(event_cfg) = &mut *event {
                                        event_cfg.on_match_reports.remove(i);
                                    }
                                },
                                "Delete"
                            }
                        }
                    }
                    button {
                        class: "outline",
                        onclick: move |_| {
                            show_report_editor.set(true);
                            on_match.set(true);
                        },
                        "Add"
                    }
                }
                div {
                    h6 { "On No Match Reports" }
                    for (i, report) in event_cfg.on_no_match_reports.iter().enumerate() {
                        details {
                            summary { "{hex::encode(report)}" }
                            button {
                                class: "danger",
                                onclick: move |_| {
                                    let mut event_signal = props.event;
                                    let mut event = event_signal.write();
                                    if let config::Event::FocusedWindowChanged(event_cfg) = &mut *event {
                                        event_cfg.on_no_match_reports.remove(i);
                                    }
                                },
                                "Delete"
                            }
                        }
                    }
                    button {
                        class: "outline",
                        onclick: move |_| {
                            show_report_editor.set(true);
                            on_match.set(false);
                        },
                        "Add"
                    }
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
                    on_submit: move || {
                        let mut event_signal = props.event;
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
        },
        if show_report_editor() {
            Dialog {
                title: "Report".to_string(),
                on_cancel: move |_| show_report_editor.set(false),
                on_ok: move |_| {
                    let mut event_signal = props.event;
                    let mut event = event_signal.write();
                    if let config::Event::FocusedWindowChanged(event_cfg) = &mut *event {
                        if on_match() {
                            event_cfg.on_match_reports.push(std::mem::take(&mut *draft_report.write()));
                        } else {
                            event_cfg.on_no_match_reports.push(std::mem::take(&mut *draft_report.write()));
                        }
                    }
                    show_report_editor.set(false);
                    draft_report.set(Vec::<u8>::default());
                },
                input {
                    name: "report",
                    value: hex::encode(draft_report()),
                    oninput: move |e| {
                        if let Ok(value) = hex::decode(e.value()) {
                            draft_report.set(value);
                        }
                    }
                }
            }
        }
    )
}
