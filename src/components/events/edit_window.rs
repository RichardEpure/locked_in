use std::path::PathBuf;

use dioxus::prelude::*;

use crate::win;

#[derive(Props, PartialEq, Clone)]
pub struct EditWindowProps {
    pub window: Signal<win::WindowMetadata>,
    pub on_submit: EventHandler<()>,
}

#[component]
pub fn EditWindow(props: EditWindowProps) -> Element {
    let mut window = props.window;

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
            class: "edit-window",
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
                onclick: move |e| {
                    e.prevent_default();
                    props.on_submit.call(());
                },
                "Submit",
            }
        }
    )
}
