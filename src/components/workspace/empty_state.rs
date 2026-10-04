use dioxus::prelude::*;
use dioxus_icons::lucide::ArrowUpRight;

#[derive(Props, Clone, PartialEq)]
pub(super) struct EmptyStateProps {
    title: &'static str,
    copy: &'static str,
}

#[component]
pub(super) fn EmptyState(props: EmptyStateProps) -> Element {
    rsx! { div { class: "empty-state", div { class: "empty-state__glyph", ArrowUpRight { size: 24, "aria-hidden": "true" } } h2 { "{props.title}" } p { "{props.copy}" } } }
}
