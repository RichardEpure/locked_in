use dioxus::prelude::*;

#[derive(Props, PartialEq, Clone)]
pub struct DialogProps {
    pub open: Option<bool>,
    pub title: String,
    pub on_ok: Option<EventHandler<()>>,
    pub on_cancel: EventHandler<()>,
    pub hide_buttons: Option<bool>,
    pub children: Element,
}

#[component]
pub fn Dialog(props: DialogProps) -> Element {
    rsx! {
        dialog {
            open: props.open.unwrap_or(true),
            article {
                header {
                    button {
                        aria_label: "Close",
                        "rel": "prev",
                        onclick: move |_| props.on_cancel.call(()),
                    }
                    p {
                        strong {
                            "{props.title}"
                        }
                    }
                }
                {props.children}
                if !props.hide_buttons.unwrap_or_default() {
                    footer {
                        button {
                            class: "secondary",
                            onclick: move |_| {
                                if let Some(on_ok) = props.on_ok {
                                    on_ok.call(());
                                }
                            },
                            "Cancel"
                        }
                        button {
                            onclick: move |_| props.on_cancel.call(()),
                            "Confirm"
                        }
                    }
                }
            }
        }
    }
}
