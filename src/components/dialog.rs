use dioxus::prelude::*;

#[derive(Props, PartialEq, Clone)]
pub struct DialogProps {
    pub title: String,
    pub on_ok: EventHandler<()>,
    pub on_cancel: EventHandler<()>,
    pub children: Element,
}

#[component]
pub fn Dialog(props: DialogProps) -> Element {
    rsx! {
        div {
            class: "dialog-backdrop",
        }
        div {
            class: "dialog",
            h2 { "{props.title}" }
            div {
                class: "dialog__content",
                {props.children}
            }
            div {
                class: "dialog__buttons",
                button {
                    class: "button button--success",
                    onclick: move |_| props.on_ok.call(()),
                    "OK"
                }
                button {
                    class: "button button--danger",
                    onclick: move |_| props.on_cancel.call(()),
                    "Cancel"
                }
            }
        }
    }
}
