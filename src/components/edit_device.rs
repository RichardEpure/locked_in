use std::path::PathBuf;

use dioxus::prelude::*;

use crate::{
    components::{dialog::Dialog, hid_devices::HidDevices},
    config,
};

#[derive(Props, PartialEq, Clone)]
pub struct EditDeviceProps {
    pub device: Signal<config::Device>,
    pub on_submit: EventHandler<()>,
}

#[component]
pub fn EditDevice(props: EditDeviceProps) -> Element {
    let mut device = props.device;

    let (name, vid, pid, usage_page, usage, report_length, report_id) = {
        let d = device.read();
        (
            d.name.clone(),
            d.vid,
            d.pid,
            d.usage_page,
            d.usage,
            d.report_length,
            d.report_id,
        )
    };

    let mut show_device_search = use_signal(|| false);

    rsx!(
        form {
            class: "edit-window",
            fieldset {
                label {
                    "Name",
                    input {
                        name: "name",
                        placeholder: "name",
                        value: "{name}",
                        oninput: move |e| {
                            let value = e.value().trim().to_string();
                            device.write().name = value;
                        }
                    }
                },
                label {
                    "Vendor ID",
                    input {
                        type: "number",
                        name: "vid",
                        placeholder: "00FF",
                        value: "{vid}",
                        oninput: move |e| {
                            let value = e.value().trim().to_string();
                            device.write().vid = value.parse::<u16>().unwrap();
                        }
                    }
                }
                label {
                    "Product ID",
                    input {
                        type: "number",
                        name: "pid",
                        placeholder: "00FF",
                        value: "{pid}",
                        oninput: move |e| {
                            let value = e.value().trim().to_string();
                            device.write().pid = value.parse::<u16>().unwrap();
                        }
                    }
                }
                label {
                    "Usage Page",
                    input {
                        type: "number",
                        name: "usage_page",
                        placeholder: "00FF",
                        value: "{usage_page}",
                        oninput: move |e| {
                            let value = e.value().trim().to_string();
                            device.write().usage_page = value.parse::<u16>().unwrap();
                        }
                    }
                }
                label {
                    "Usage",
                    input {
                        type: "number",
                        name: "usage",
                        placeholder: "00FF",
                        value: "{usage}",
                        oninput: move |e| {
                            let value = e.value().trim().to_string();
                            device.write().usage_page = value.parse::<u16>().unwrap();
                        }
                    }
                }
                label {
                    "Report Length",
                    input {
                        type: "number",
                        name: "report_length",
                        placeholder: "32",
                        value: "{report_length}",
                        oninput: move |e| {
                            let value = e.value().trim().to_string();
                            device.write().report_length = value.parse::<u16>().unwrap();
                        }
                    }
                }
                label {
                    "Report ID",
                    input {
                        type: "number",
                        name: "report_id",
                        placeholder: "00",
                        value: "{report_id}",
                        oninput: move |e| {
                            let value = e.value().trim().to_string();
                            device.write().report_id = u8::from_str_radix(&value, 16).unwrap();
                        }
                    }
                }
            },
            div {
                class: "grid",
                input {
                    type: "button",
                    value: "Search",
                    onclick: move |_| {
                        show_device_search.set(true);
                    },
                }
                input {
                    type: "submit",
                    onclick: move |_| {
                        props.on_submit.call(());
                    }
                }
            }
        },
        if show_device_search() {
            Dialog {
                title: "Connected Devices".to_string(),
                hide_buttons: true,
                on_cancel: move |_| show_device_search.set(false),
                HidDevices {
                    device: device,
                    on_submit: move |_| {
                        show_device_search.set(false);
                    },
                }
            }
        }
    )
}
