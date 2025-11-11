use dioxus::prelude::*;

use crate::hid;

#[component]
pub fn HidDevices() -> Element {
    let devices = use_signal(hid::get_devices);

    rsx! {
        div {
            class: "hid-devices",
            ul {
                class: "hid-devices__list",
                for device in devices.read().clone() {
                    li {
                        "{device.vendor_id} - ",
                        "{device.product_id} - ",
                        "{device.manufacturer_string} - ",
                        "{device.product_string}"
                        for usage in device.usages {
                            div {
                                "Usage Page: {usage.usage_page}, Usage: {usage.usage}"
                            }
                        }
                    }
                }
            }
        }
    }
}
