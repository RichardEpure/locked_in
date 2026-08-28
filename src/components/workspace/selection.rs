use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub(super) struct SelectionProps {
    pub(super) selected: Signal<Option<String>>,
}
