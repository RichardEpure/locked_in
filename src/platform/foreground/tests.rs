use std::{
    path::PathBuf,
    sync::{Arc, Barrier},
    thread,
};

use super::*;

struct TestSource {
    publisher: Arc<Publisher<u64>>,
}

impl TestSource {
    fn new() -> Self {
        Self {
            publisher: Arc::new(Publisher::new(Foreground::new())),
        }
    }

    fn foreground(&self) -> Foreground {
        self.publisher.foreground.clone()
    }

    fn observe(&self, identity: u64, metadata: FocusedWindow) {
        if let Some(pending) = self.pending(identity) {
            pending.resolve(Some(metadata));
        }
    }

    fn sample(&self, identity: u64, metadata: FocusedWindow) {
        if let Some((ticket, metadata)) = self.publisher.begin_with(|| Some((identity, metadata))) {
            self.publisher.complete(ticket, Some(metadata));
        }
    }

    fn pending(&self, identity: u64) -> Option<PendingMetadata> {
        Some(PendingMetadata {
            publisher: self.publisher.clone(),
            ticket: self.publisher.begin(identity)?,
        })
    }
}

struct PendingMetadata {
    publisher: Arc<Publisher<u64>>,
    ticket: Ticket,
}

impl PendingMetadata {
    fn resolve(self, metadata: Option<FocusedWindow>) {
        self.publisher.complete(self.ticket, metadata);
    }
}

fn window(title: &str) -> FocusedWindow {
    FocusedWindow {
        title: Some(title.to_string()),
        class: Some("WindowClass".to_string()),
        exe: Some(PathBuf::from("app.exe")),
    }
}

#[test]
fn startup_publishes_the_current_window_without_a_callback() {
    let source = TestSource::new();
    let foreground = source.foreground();
    let observations = foreground.subscribe();

    start_monitor(|| Ok(()), || source.sample(10, window("already focused"))).unwrap();

    let current = foreground.current();
    assert_eq!(current.generation, 1);
    assert_eq!(current.window, window("already focused"));
    assert_eq!(*observations.borrow(), current);
}

#[test]
fn rapid_observations_retain_the_latest_generation() {
    let source = TestSource::new();
    let foreground = source.foreground();
    let observations = foreground.subscribe();

    source.observe(10, window("A"));
    source.observe(20, window("B"));

    let latest = observations.borrow().clone();
    assert_eq!(latest.generation, 2);
    assert_eq!(latest.window, window("B"));
    assert_eq!(foreground.current(), latest);
}

#[test]
fn consecutive_duplicate_identity_is_suppressed_even_if_metadata_changes() {
    let source = TestSource::new();
    let foreground = source.foreground();

    source.observe(10, window("first"));
    source.observe(10, window("changed"));

    let latest = foreground.current();
    assert_eq!(latest.generation, 1);
    assert_eq!(latest.window, window("first"));
}

#[test]
fn filtered_observation_breaks_duplicate_identity_without_publication() {
    let source = TestSource::new();
    let foreground = source.foreground();
    let mut observations = foreground.subscribe();

    source.observe(10, window("A"));
    observations.borrow_and_update();
    let older = source.pending(30).unwrap();
    source.pending(20).unwrap().resolve(None);
    older.resolve(Some(window("stale before filtered window")));
    assert!(!observations.has_changed().unwrap());
    source.observe(10, window("A"));

    let latest = foreground.current();
    assert_eq!(latest.generation, 2);
    assert_eq!(latest.window, window("A"));
}

#[test]
fn identical_metadata_from_distinct_windows_advances_generation() {
    let source = TestSource::new();
    let foreground = source.foreground();
    let facts = window("same");

    source.observe(10, facts.clone());
    source.observe(20, facts.clone());

    let latest = foreground.current();
    assert_eq!(latest.generation, 2);
    assert_eq!(latest.window, facts);
}

#[test]
fn concurrent_startup_and_callback_for_same_window_publish_once() {
    let source = Arc::new(TestSource::new());
    let foreground = source.foreground();
    let barrier = Arc::new(Barrier::new(3));
    let startup = {
        let source = source.clone();
        let barrier = barrier.clone();
        thread::spawn(move || {
            start_monitor(
                || Ok(()),
                || {
                    barrier.wait();
                    source.sample(10, window("A"));
                },
            )
            .unwrap();
        })
    };
    let callback = {
        let source = source.clone();
        let barrier = barrier.clone();
        thread::spawn(move || {
            barrier.wait();
            source.observe(10, window("A"));
        })
    };
    barrier.wait();
    startup.join().unwrap();
    callback.join().unwrap();

    assert_eq!(foreground.current().generation, 1);
    assert_eq!(foreground.current().window, window("A"));
}

#[test]
fn accepted_observations_project_metadata_for_existing_consumers() {
    let source = TestSource::new();
    let foreground = source.foreground();
    let facts = window("projected");

    source.observe(10, facts.clone());

    assert_eq!(foreground.current().window, facts);
}

#[test]
fn partial_metadata_is_retained_with_its_generation() {
    let source = TestSource::new();
    let foreground = source.foreground();
    let partial = FocusedWindow {
        class: Some("PartialWindow".to_string()),
        ..FocusedWindow::default()
    };

    source.observe(10, partial.clone());

    let latest = foreground.current();
    assert_eq!(latest.generation, 1);
    assert_eq!(latest.window, partial);
}

#[test]
fn older_startup_completion_cannot_overwrite_a_newer_callback() {
    let source = TestSource::new();
    let foreground = source.foreground();
    let startup = source.pending(10).unwrap();
    let callback = source.pending(20).unwrap();

    callback.resolve(Some(window("callback")));
    startup.resolve(Some(window("startup")));

    let latest = foreground.current();
    assert_eq!(latest.generation, 1);
    assert_eq!(latest.window, window("callback"));
}

#[test]
fn current_and_subscribed_observations_keep_generation_and_metadata_together() {
    let source = TestSource::new();
    let foreground = source.foreground();
    let mut observations = foreground.subscribe();
    let pending = source.pending(10).unwrap();
    let facts = window("ordered");

    pending.resolve(Some(facts.clone()));

    assert!(observations.has_changed().unwrap());
    let published = observations.borrow_and_update().clone();
    assert_eq!(published.generation, 1);
    assert_eq!(published.window, facts);
    assert_eq!(foreground.current(), published);
    assert_eq!(*foreground.subscribe().borrow(), published);
}
