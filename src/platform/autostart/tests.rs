use super::*;

#[test]
fn apply_failure_uses_the_confirmed_native_state() {
    let outcome = reconcile(true, |_| anyhow::bail!("access denied"), || Ok(false));
    assert_eq!(outcome.state, LaunchAtLoginState::Confirmed(false));
    assert!(
        outcome
            .warning
            .as_deref()
            .is_some_and(|warning| warning.contains("access denied"))
    );
}

#[test]
fn apply_and_inspection_failure_is_unconfirmed() {
    let outcome = reconcile(
        false,
        |_| anyhow::bail!("delete failed"),
        || anyhow::bail!("query failed"),
    );
    assert_eq!(outcome.state, LaunchAtLoginState::Unconfirmed);
    assert!(outcome.warning.as_deref().is_some_and(|warning| {
        warning.contains("delete failed") && warning.contains("query failed")
    }));
}
