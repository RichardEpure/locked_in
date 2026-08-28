use super::*;

#[test]
fn mixed_test_feedback_reports_successes_and_failures() {
    let feedback = test_feedback(TestDispatchResult {
        sent: 1,
        failures: vec!["Deck: disconnected".into(), "Pad: ambiguous".into()],
    });

    assert_eq!(
        feedback,
        (
            false,
            "Sent to 1 device; 2 failed: Deck: disconnected; Pad: ambiguous".into()
        )
    );
}
