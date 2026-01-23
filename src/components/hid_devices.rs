use dioxus::prelude::*;

use crate::{config, hid};

#[derive(Props, PartialEq, Clone)]
pub struct HidDevicesProps {
    pub device: Signal<config::Device>,
    pub on_submit: EventHandler<()>,
}

#[component]
pub fn HidDevices(props: HidDevicesProps) -> Element {
    let mut device = props.device;
    let devices = use_signal(|| {
        hid::HID_DEVICES
            .lock()
            .expect("Could not fetch HID device list")
            .refresh()
            .get_metadata_list()
    });
    let mut hid_device = use_signal(|| None::<hid::HidMetadata>);
    let mut usage_pair = use_signal(|| None::<hid::UsagePair>);

    rsx! {
        form {
            class: "hid-devices",
            for device in devices.read().iter() {
                details {
                    class: "hid-device",
                    summary { "{device.manufacturer_string} - {device.product_string}" }
                    h6 { "PID: {device.product_id}" },
                    h6 { "VID: {device.vendor_id}" },
                    fieldset {
                        class: "hid-device__usages",
                        legend { "Usages" }
                        for usage in device.usages.iter() {
                            label {
                                input {
                                    type: "radio",
                                    name: "usage",
                                    onchange: {
                                        let mut device = device.clone();
                                        let mut usage = usage.clone();
                                        move |_| {
                                            hid_device.set(Some(std::mem::take(&mut device)));
                                            usage_pair.set(Some(std::mem::take(&mut usage)));
                                        }
                                    }
                                }
                                "Usage Page: {usage.usage_page} - Usage: {usage.usage}"
                            }
                        }
                    }
                }
            }
            input {
                type: "submit",
                onclick: move |_| {
                    if let Some(hid_device) = &*hid_device.read()
                        && let Some(usage_pair) = &*usage_pair.read()
                    {
                        device.set(config::Device {
                            name: if !hid_device.product_string.is_empty() {
                                hid_device.product_string.clone()
                            } else if !hid_device.manufacturer_string.is_empty() {
                                hid_device.manufacturer_string.clone()
                            } else {
                                "Untitled".to_string()
                            },
                            vid: hid_device.vendor_id,
                            pid: hid_device.product_id,
                            usage_page: usage_pair.usage_page,
                            usage: usage_pair.usage,
                            report_length: u16::default(),
                            report_id: u8::default(),
                        });
                    }
                    props.on_submit.call(());
                }
            }
        }
    }
}
