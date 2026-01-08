use dioxus::prelude::*;

use crate::{
    components::{dialog::Dialog, edit_device::EditDevice},
    config,
};

#[derive(Debug, Copy, Clone)]
struct EditTarget {
    index: Option<usize>, // None = "Add", Some(i) = "Edit existing i"
}

#[derive(Props, PartialEq, Clone)]
pub struct DevicesProps {
    pub devices: Signal<Vec<config::Device>>,
}

#[component]
pub fn Devices(props: DevicesProps) -> Element {
    let mut devices_signal = props.devices;
    let mut show_device_editor = use_signal(|| false);
    let mut draft_device = use_signal(config::Device::default);
    let mut edit_target = use_signal(|| None::<EditTarget>);

    rsx!(
        div {
            class: "devices",
            for (i, device) in props.devices.read().iter().enumerate() {
                details {
                    class: "device",
                    summary { "{device.name}" }
                    ul {
                        li { "{device.vid}" }
                        li { "{device.pid}" }
                        li { "{device.usage_page}" }
                        li { "{device.usage}" }
                        li { "{device.report_length}" }
                        li { "{device.report_id}" }
                    }
                    div {
                        role: "group",
                        class: "device__buttons",
                        button {
                            class: "outline",
                            onclick: {
                                let mut device = device.clone();
                                move |_| {
                                    edit_target.set(Some(EditTarget { index: Some(i) }));
                                    draft_device.set(std::mem::take(&mut device));
                                    show_device_editor.set(true);
                                }
                            },
                            "Edit"
                        },
                        button {
                            class: "danger",
                            onclick: move |_| {
                                devices_signal.write().remove(i);
                            },
                            "Delete"
                        }
                    }
                }
            }
            button {
                class: "outline",
                onclick: move |_| {
                    edit_target.set(Some(EditTarget { index: None }));
                    show_device_editor.set(true);
                },
                "Add"
            }
        },
        if show_device_editor() {
            Dialog {
                title: "Device".to_string(),
                hide_buttons: true,
                on_cancel: move |_| show_device_editor.set(false),
                EditDevice {
                    device: draft_device,
                    on_submit: move || {
                        let Some(target) = *edit_target.read() else {
                            show_device_editor.set(false);
                            return;
                        };
                        let new_device = std::mem::take(&mut *draft_device.write());
                        if let Some(i) = target.index {
                            devices_signal.write()[i] = new_device;
                        } else {
                            devices_signal.write().push(new_device);
                        }
                        edit_target.set(None);
                        show_device_editor.set(false);
                    }
                }
            }
        }
    )
}
