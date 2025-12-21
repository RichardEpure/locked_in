use dioxus::prelude::*;

use crate::hid;

#[derive(Props, PartialEq, Clone)]
pub struct HidDevicesProps {
    pub on_select: EventHandler<hid::HidMetadata>,
}

#[component]
pub fn HidDevices(props: HidDevicesProps) -> Element {
    let devices = use_signal(hid::get_devices);
    let mut selected = use_signal(|| None as Option<u16>); // HidMetadata product_id

    rsx! {
        div {
            class: "hid-devices",
            ul {
                class: "hid-devices__list",
                for device in devices.read().clone() {
                    li {
                        class: "hid-devices__item",
                        class: if *selected.read() == Some(device.product_id) {
                            "hid-devices__item--selected"
                        },
                        onclick: move |_| {
                            // props.on_select.call(device.clone());
                            selected.set(Some(device.product_id));
                        },
                        "{device.manufacturer_string} ",
                        "{device.product_string}"
                        if *selected.read() == Some(device.product_id) {
                            ul {
                                class: "hid-devices__item-usages",
                                for usage in device.usages.iter() {
                                    li {
                                        class: "hid-devices__item-usage",
                                        "Usage Page: {usage.usage_page}, Usage: {usage.usage}"
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
