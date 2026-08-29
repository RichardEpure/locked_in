use super::*;

fn target(automation: &str) -> Option<CaptureTarget> {
    Some(CaptureTarget::new(
        automation.into(),
        "case-1".into(),
        false,
    ))
}

#[test]
fn delayed_capture_from_old_generation_is_rejected_after_rearm() {
    let old_target = target("old");
    let new_target = target("new");

    assert!(!capture_session_is_current(
        4,
        &old_target,
        5,
        &new_target,
        true
    ));
    assert!(capture_session_is_current(
        5,
        &new_target,
        5,
        &new_target,
        true
    ));
}

#[test]
fn delayed_capture_is_rejected_after_cancel_even_before_unmount() {
    let old_target = target("old");

    assert!(!capture_session_is_current(8, &old_target, 9, &None, false));
}
