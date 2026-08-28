use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub(super) struct NumericFieldProps {
    label: &'static str,
    value: u16,
    on_change: EventHandler<u16>,
}

#[component]
pub(super) fn NumericField(props: NumericFieldProps) -> Element {
    rsx! { label { "{props.label}" input { type: "number", min: "0", max: "65535", value: "{props.value}", oninput: move |event| if let Ok(value) = event.value().parse() { props.on_change.call(value) } } } }
}
