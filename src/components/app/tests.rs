use std::cell::RefCell;

use super::*;

#[test]
fn focused_window_bridge_projects_the_current_value_before_waiting_for_changes() {
    let current = win::WindowMetadata {
        title: Some("already focused".to_string()),
        ..win::WindowMetadata::default()
    };
    let (publisher, mut receiver) = tokio::sync::watch::channel(win::WindowMetadata::default());
    publisher.send_replace(current.clone());
    let projected = RefCell::new(None);

    publish_current_focused_window(&mut receiver, |focused| {
        *projected.borrow_mut() = Some(focused);
    });

    assert_eq!(projected.into_inner(), Some(current));
    assert!(!receiver.has_changed().unwrap());
}
