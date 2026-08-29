use std::{
    cell::Cell,
    path::PathBuf,
    sync::{Arc, Barrier},
    thread,
};

use super::{
    ALT_TAB_HOST_CLASS, ForegroundPublisher, WindowMetadata, startup_registry_value_exists,
};

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

fn window(title: &str) -> WindowMetadata {
    WindowMetadata {
        title: Some(title.to_string()),
        class: Some("WindowClass".to_string()),
        exe: Some(PathBuf::from("app.exe")),
    }
}

fn observe(
    publisher: &ForegroundPublisher,
    raw_hwnd: isize,
    window: WindowMetadata,
) -> Option<super::ForegroundObservation> {
    let ticket = publisher.begin(raw_hwnd)?;
    publisher.complete(ticket, window)
}

#[test]
fn rapid_observations_retain_the_latest_generation() {
    let publisher = ForegroundPublisher::new();
    let observations = publisher.subscribe_observations();

    observe(&publisher, 10, window("A"));
    observe(&publisher, 20, window("B"));

    let latest = observations.borrow().clone();
    assert_eq!(latest.generation, 2);
    assert_eq!(latest.raw_hwnd, 20);
    assert_eq!(latest.window, window("B"));
}

#[test]
fn consecutive_duplicate_hwnd_is_suppressed_even_if_metadata_changes() {
    let publisher = ForegroundPublisher::new();

    assert!(observe(&publisher, 10, window("first")).is_some());
    assert!(observe(&publisher, 10, window("changed")).is_none());

    let latest = publisher.subscribe_observations().borrow().clone();
    assert_eq!(latest.generation, 1);
    assert_eq!(latest.window, window("first"));
}

#[test]
fn ignored_alt_tab_host_breaks_duplicate_identity_without_publication() {
    let publisher = ForegroundPublisher::new();
    let mut observations = publisher.subscribe_observations();
    let mut metadata = publisher.subscribe_metadata();

    observe(&publisher, 10, window("A"));
    observations.borrow_and_update();
    metadata.borrow_and_update();
    assert!(
        observe(
            &publisher,
            20,
            WindowMetadata {
                class: Some(ALT_TAB_HOST_CLASS.to_string()),
                ..WindowMetadata::default()
            }
        )
        .is_none()
    );
    assert!(!observations.has_changed().unwrap());
    assert!(!metadata.has_changed().unwrap());
    observe(&publisher, 10, window("A"));

    let latest = observations.borrow().clone();
    assert_eq!(latest.generation, 2);
    assert_eq!(latest.raw_hwnd, 10);
    assert_eq!(latest.window, window("A"));
}

#[test]
fn identical_metadata_from_distinct_hwnds_advances_generation() {
    let publisher = ForegroundPublisher::new();
    let facts = window("same");

    observe(&publisher, 10, facts.clone());
    observe(&publisher, 20, facts.clone());

    let latest = publisher.subscribe_observations().borrow().clone();
    assert_eq!(latest.generation, 2);
    assert_eq!(latest.raw_hwnd, 20);
    assert_eq!(latest.window, facts);
}

#[test]
fn concurrent_startup_and_callback_for_same_hwnd_publish_once() {
    let publisher = Arc::new(ForegroundPublisher::new());
    let barrier = Arc::new(Barrier::new(3));
    let mut threads = Vec::new();

    for _ in 0..2 {
        let publisher = publisher.clone();
        let barrier = barrier.clone();
        threads.push(thread::spawn(move || {
            barrier.wait();
            observe(&publisher, 10, window("A"));
        }));
    }

    barrier.wait();
    for thread in threads {
        thread.join().unwrap();
    }

    let latest = publisher.subscribe_observations().borrow().clone();
    assert_eq!(latest.generation, 1);
}

#[test]
fn accepted_observations_project_metadata_for_existing_consumers() {
    let publisher = ForegroundPublisher::new();
    let metadata = publisher.subscribe_metadata();
    let facts = window("projected");

    observe(&publisher, 10, facts.clone());

    assert_eq!(*metadata.borrow(), facts);
}

#[test]
fn partial_metadata_is_retained_with_identity_and_generation() {
    let publisher = ForegroundPublisher::new();
    let partial = WindowMetadata {
        class: Some("PartialWindow".to_string()),
        ..WindowMetadata::default()
    };

    observe(&publisher, 10, partial.clone());

    let latest = publisher.subscribe_observations().borrow().clone();
    assert_eq!(latest.generation, 1);
    assert_eq!(latest.raw_hwnd, 10);
    assert_eq!(latest.window, partial);
}

#[test]
fn older_startup_completion_cannot_overwrite_a_newer_callback() {
    let publisher = ForegroundPublisher::new();
    let startup = publisher.begin(10).unwrap();
    let callback = publisher.begin(20).unwrap();

    let published = publisher.complete(callback, window("callback")).unwrap();
    assert_eq!(published.generation, 1);
    assert!(publisher.complete(startup, window("startup")).is_none());

    let latest = publisher.subscribe_observations().borrow().clone();
    assert_eq!(latest.generation, 1);
    assert_eq!(latest.raw_hwnd, 20);
    assert_eq!(latest.window, window("callback"));
}

#[test]
fn metadata_is_published_before_the_versioned_observation() {
    let publisher = ForegroundPublisher::new();
    let mut metadata = publisher.subscribe_metadata();
    let mut observations = publisher.subscribe_observations();
    let ticket = publisher.begin(10).unwrap();
    let facts = window("ordered");

    publisher.complete_before_versioned(ticket, facts.clone(), || {
        assert!(metadata.has_changed().unwrap());
        assert_eq!(*metadata.borrow_and_update(), facts);
        assert!(!observations.has_changed().unwrap());
    });

    assert!(observations.has_changed().unwrap());
    let published = observations.borrow_and_update().clone();
    assert_eq!(published.generation, 1);
    assert_eq!(published.window, facts);
}
