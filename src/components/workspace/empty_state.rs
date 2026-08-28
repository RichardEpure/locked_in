use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub(super) struct EmptyStateProps {
    title: &'static str,
    copy: &'static str,
}

#[component]
pub(super) fn EmptyState(props: EmptyStateProps) -> Element {
    rsx! { div { class: "empty-state", div { class: "empty-state__glyph", "↗" } h2 { "{props.title}" } p { "{props.copy}" } } }
}
