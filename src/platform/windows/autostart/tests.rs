use std::cell::Cell;

use super::startup_registry_value_exists;

#[test]
fn absent_startup_registry_key_is_confirmed_disabled_without_querying_a_value() {
    let queried = Cell::new(false);
    let enabled = startup_registry_value_exists(
        || Ok(None::<()>),
        |_| {
            queried.set(true);
            Ok(true)
        },
    )
    .unwrap();
    assert!(!enabled);
    assert!(!queried.get());
}

#[test]
fn absent_startup_registry_value_is_confirmed_disabled() {
    let enabled = startup_registry_value_exists(|| Ok(Some(())), |_| Ok(false)).unwrap();
    assert!(!enabled);
}

#[test]
fn present_startup_registry_value_is_confirmed_enabled() {
    let enabled = startup_registry_value_exists(|| Ok(Some(())), |_| Ok(true)).unwrap();
    assert!(enabled);
}

#[test]
fn startup_registry_query_errors_are_not_reported_as_disabled() {
    let open_error =
        startup_registry_value_exists::<()>(|| anyhow::bail!("access denied"), |_| Ok(false))
            .unwrap_err();
    let value_error =
        startup_registry_value_exists(|| Ok(Some(())), |_| anyhow::bail!("value query failed"))
            .unwrap_err();
    assert!(open_error.to_string().contains("access denied"));
    assert!(value_error.to_string().contains("value query failed"));
}
