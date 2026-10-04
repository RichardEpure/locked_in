use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum NumericFormat {
    Decimal,
    Hexadecimal { width: usize },
}

#[derive(Props, Clone, PartialEq)]
pub(super) struct NumericFieldProps {
    label: &'static str,
    value: u16,
    max: u16,
    format: NumericFormat,
    on_change: EventHandler<u16>,
}

#[component]
pub(super) fn NumericField(props: NumericFieldProps) -> Element {
    let format = props.format;
    let max = props.max;
    let value = props.value;
    let on_change = props.on_change;
    let mut text = use_signal(|| format_numeric_value(value, format));
    let mut current_value = use_signal(|| value);

    use_effect(use_reactive((&value, &format), move |(value, format)| {
        if value != *current_value.peek() {
            current_value.set(value);
            text.set(format_numeric_value(value, format));
        }
    }));

    rsx! {
        label {
            "{props.label}"
            match format {
                NumericFormat::Decimal => rsx! {
                    input {
                        type: "number",
                        min: "0",
                        max: "{max}",
                        value: "{value}",
                        oninput: move |event| {
                            if let Some(value) = parse_numeric_value(&event.value(), format, max) {
                                on_change.call(value);
                            }
                        }
                    }
                },
                NumericFormat::Hexadecimal { width } => rsx! {
                    input {
                        type: "text",
                        inputmode: "text",
                        maxlength: "{width}",
                        pattern: "[0-9A-Fa-f]*",
                        autocomplete: "off",
                        spellcheck: "false",
                        value: "{text}",
                        oninput: move |event| {
                            let input = sanitize_hex_input(&event.value(), width);
                            text.set(input.clone());
                            if let Some(value) = parse_numeric_value(&input, format, max) {
                                current_value.set(value);
                                on_change.call(value);
                            }
                        },
                        onblur: move |_| {
                            text.set(format_numeric_value(*current_value.peek(), format));
                        }
                    }
                },
            }
        }
    }
}

fn format_numeric_value(value: u16, format: NumericFormat) -> String {
    match format {
        NumericFormat::Decimal => value.to_string(),
        NumericFormat::Hexadecimal { width } => format!("{value:0width$X}"),
    }
}

fn parse_numeric_value(value: &str, format: NumericFormat, max: u16) -> Option<u16> {
    let parsed = match format {
        NumericFormat::Decimal => value.parse().ok(),
        NumericFormat::Hexadecimal { .. } => u16::from_str_radix(value, 16).ok(),
    }?;
    (parsed <= max).then_some(parsed)
}

fn sanitize_hex_input(value: &str, width: usize) -> String {
    value
        .chars()
        .filter(char::is_ascii_hexdigit)
        .take(width)
        .flat_map(char::to_uppercase)
        .collect()
}

#[cfg(test)]
mod tests;
