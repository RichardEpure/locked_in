use dioxus::prelude::*;

#[derive(Props, PartialEq, Clone)]
pub struct DialogProps {
    pub open: bool,
    pub title: String,
    pub on_ok: EventHandler<()>,
    pub on_cancel: EventHandler<()>,
    pub hide_buttons: Option<bool>,
    pub children: Element,
}

#[component]
pub fn Dialog(props: DialogProps) -> Element {
    rsx! {
        dialog {
            open: props.open,
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
                            onclick: move |_| props.on_ok.call(()),
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
