use super::*;

fn target(id: &str) -> CaptureTarget {
    CaptureTarget::new(id.into(), "case-1".into(), false)
}

fn window() -> FocusedWindow {
    FocusedWindow {
        title: Some("Captured".into()),
        ..FocusedWindow::default()
    }
}

#[test]
fn rearm_rejects_old_shortcut_timeout_and_cancel_including_same_target() {
    let mut session = CaptureSession::default();
    let old = session.arm(Some(target("editor")));
    let current = session.arm(Some(target("editor")));
    assert!(!session.capture(old, window()));
    session.expire(old);
    session.cancel(old);
    assert!(session.is_armed());
    assert_eq!(session.generation(), current);
    assert!(session.capture(current, window()));
}

#[test]
fn cancel_rejects_a_callback_even_before_shortcut_unmount() {
    let mut session = CaptureSession::default();
    let generation = session.arm(Some(target("editor")));
    session.cancel(generation);
    assert!(!session.capture(generation, window()));
    assert!(!session.is_armed());
    assert!(session.captured().is_none());
    assert!(session.target().is_none());
}

#[test]
fn capture_is_accepted_once_and_survives_its_armed_timeout() {
    let mut session = CaptureSession::default();
    let generation = session.arm(Some(target("editor")));
    assert!(session.capture(generation, window()));
    assert!(!session.capture(generation, FocusedWindow::default()));
    session.expire(generation);
    assert!(!session.is_armed());
    assert_eq!(session.captured().unwrap().window, window());
    assert_eq!(session.take_targeted(generation, &target("other")), None);
    assert_eq!(
        session.take_targeted(generation, &target("editor")),
        Some(window())
    );
    assert_eq!(session.take_targeted(generation, &target("editor")), None);
    assert!(session.target().is_none());
}

#[test]
fn targeted_and_untargeted_sessions_do_not_leak_results_or_navigation_locks() {
    let mut session = CaptureSession::default();
    let targeted = session.arm(Some(target("editor")));
    session.capture(targeted, window());
    let untargeted = session.arm(None);
    assert!(session.target().is_none());
    assert!(session.captured().is_none());
    assert_eq!(session.take_targeted(targeted, &target("editor")), None);
    assert!(session.capture(untargeted, window()));
    assert_eq!(session.take_targeted(untargeted, &target("editor")), None);
    assert!(session.captured().is_some());
    let next = session.arm(Some(target("next")));
    session.cancel(untargeted);
    assert_eq!(session.target(), Some(&target("next")));
    session.expire(next);
    assert!(session.target().is_none());
    assert!(!session.capture(next, window()));
}

#[test]
fn removing_a_target_cancels_armed_or_captured_work_and_retains_feedback_after_unmount() {
    for captured in [false, true] {
        let mut session = CaptureSession::default();
        let generation = session.arm(Some(target("removed")));
        if captured {
            assert!(session.capture(generation, window()));
        }
        session.target_removed(generation);
        assert!(session.cancellation_message().is_some());
        assert!(session.target().is_none());
        assert!(session.captured().is_none());
        assert!(!session.capture(generation, window()));
        session.cancel(generation); // Old editor cleanup cannot erase the feedback.
        assert!(session.cancellation_message().is_some());
        let replacement = session.arm(Some(target("replacement")));
        session.target_removed(generation);
        assert!(session.cancellation_message().is_none());
        assert_eq!(session.generation(), replacement);
        assert_eq!(session.target(), Some(&target("replacement")));
    }
}
