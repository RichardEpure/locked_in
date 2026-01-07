use std::ops::Deref;

use dioxus::prelude::*;

use crate::{
    components::events::focused_window_changed::FocusedWindowChanged,
    config::{self},
};

#[derive(Props, PartialEq, Clone)]
pub struct EventConfiguratorProps {
    pub event: Signal<config::Event>,
}

#[component]
pub fn EventConfigurator(props: EventConfiguratorProps) -> Element {
    match props.event.read().deref() {
        config::Event::FocusedWindowChanged(_) => {
            rsx!(FocusedWindowChanged { event: props.event })
        }
    }
}
