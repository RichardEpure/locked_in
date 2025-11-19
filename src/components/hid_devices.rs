use dioxus::prelude::*;

use crate::hid;

#[derive(Props, PartialEq, Clone)]
pub struct HidDevicesProps {
    pub on_select: EventHandler<hid::HidMetadata>,
}

#[component]
pub fn HidDevices(props: HidDevicesProps) -> Element {
    let devices = use_signal(hid::get_devices);

    rsx! {
        div {
            class: "hid-devices",
            ul {
                class: "hid-devices__list",
                for device in devices.read().clone() {
                    li {
                        class: "hid-devices__item",
                        onclick: move |_| {
                            props.on_select.call(device.clone());
                        },
                        "{device.manufacturer_string} ",
                        "{device.product_string}"
                        // for usage in device.usages.iter() {
                        //     div {
                        //         "Usage Page: {usage.usage_page}, Usage: {usage.usage}"
                        //     }
                        // }
                    }
                }
            }
        }
    }
}
