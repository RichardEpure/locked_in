use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

use super::*;

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct CountingStartup {
    calls: AtomicUsize,
}

#[test]
fn startup_apply_failure_uses_the_confirmed_registry_state() {
    let outcome =
        reconcile_start_with_windows(true, |_| anyhow::bail!("access denied"), || Ok(false));

    assert_eq!(
        outcome.state,
        config::StartWithWindowsState::Confirmed(false)
    );
    assert!(
        outcome
            .warning
            .as_deref()
            .is_some_and(|warning| warning.contains("access denied"))
    );
}

#[test]
fn startup_apply_and_registry_query_failure_is_unconfirmed() {
    let outcome = reconcile_start_with_windows(
        false,
        |_| anyhow::bail!("delete failed"),
        || anyhow::bail!("query failed"),
    );

    assert_eq!(outcome.state, config::StartWithWindowsState::Unconfirmed);
    assert!(outcome.warning.as_deref().is_some_and(|warning| {
        warning.contains("delete failed") && warning.contains("query failed")
    }));
}

impl config::StartWithWindows for CountingStartup {
    fn reconcile(&self, desired: bool) -> config::StartWithWindowsOutcome {
        self.calls.fetch_add(1, Ordering::SeqCst);
        config::StartWithWindowsOutcome::confirmed(desired)
    }
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "locked-in-main-bootstrap-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn strict_load_failure_is_visible_non_destructive_and_skips_startup_registration() {
    let directory = TestDirectory::new();
    let config_path = directory.0.join("config.toml");
    let invalid_source = "version = 1\n[settings]\nstart_with_windows = true\n";
    fs::write(&config_path, invalid_source).unwrap();
    let store = Arc::new(config::ConfigStore::new(&config_path));
    let startup = Arc::new(CountingStartup {
        calls: AtomicUsize::new(0),
    });

    let loaded = load_initial_configuration(store, startup.clone());

    assert!(loaded.coordinator.is_none());
    assert!(loaded.publication.is_none());
    assert_eq!(loaded.bootstrap.ui, config::Config::default());
    assert_eq!(loaded.bootstrap.revision, 0);
    assert!(loaded.bootstrap.error.as_deref().is_some_and(|error| {
        error.contains("Configuration could not be loaded")
            && error.contains("unsupported config version")
    }));
    assert_eq!(startup.calls.load(Ordering::SeqCst), 0);
    assert_eq!(fs::read_to_string(config_path).unwrap(), invalid_source);
    assert!(initial_window_visible(
        false,
        loaded.bootstrap.error.as_deref(),
        &loaded.bootstrap.ui.settings,
    ));
}

#[test]
fn directory_creation_failure_has_a_path_specific_bootstrap_error() {
    let directory = TestDirectory::new();
    let file = directory.0.join("not-a-directory");
    fs::write(&file, "occupied").unwrap();

    let error = create_directory(&file).unwrap_err().to_string();

    assert!(error.contains("failed to create application directory"));
    assert!(error.contains("not-a-directory"));
}

#[test]
fn preferred_path_failure_uses_one_coherent_fallback_without_touching_config() {
    let directory = TestDirectory::new();
    let blocked_preferred = directory.0.join("blocked-preferred-root");
    fs::write(&blocked_preferred, "original source sentinel").unwrap();
    let preferred = config::ApplicationPaths::from_data_root(&blocked_preferred);
    let fallback = config::ApplicationPaths::from_data_root(directory.0.join("fallback-root"));

    let prepared = prepare_application_paths_from(Ok(preferred), fallback.clone()).unwrap();

    assert_eq!(prepared.paths, fallback);
    assert!(prepared.bootstrap_error.as_deref().is_some_and(|error| {
        error.contains("Using temporary diagnostics and WebView root")
            && error.contains("Configuration was not loaded")
            && error.contains("fallback-root")
    }));
    assert!(prepared.paths.data_root().is_dir());
    assert!(prepared.paths.log_directory().is_dir());
    assert!(prepared.paths.webview_data_directory().is_dir());
    assert!(!prepared.paths.config_path().exists());
    assert_eq!(
        fs::read_to_string(blocked_preferred).unwrap(),
        "original source sentinel"
    );
}

#[test]
fn fallback_bootstrap_never_invokes_configuration_loading() {
    let directory = TestDirectory::new();
    let fallback = config::ApplicationPaths::from_data_root(directory.0.join("fallback-root"));
    let prepared = prepare_application_paths_from(
        Err(anyhow!("preferred root failed before configuration access")),
        fallback,
    )
    .unwrap();
    let loaded = AtomicBool::new(false);

    let initial = initialize_configuration(&prepared, |_| {
        loaded.store(true, Ordering::SeqCst);
        panic!("fallback must not load configuration")
    });

    assert!(!loaded.load(Ordering::SeqCst));
    assert!(initial.coordinator.is_none());
    assert!(initial.publication.is_none());
    assert!(initial.bootstrap.error.is_some());
    assert!(!prepared.paths.config_path().exists());
}

#[test]
fn preferred_and_fallback_path_failures_abort_with_both_causes() {
    let directory = TestDirectory::new();
    let blocked_fallback = directory.0.join("blocked-fallback-root");
    fs::write(&blocked_fallback, "occupied").unwrap();
    let fallback = config::ApplicationPaths::from_data_root(&blocked_fallback);

    let error = prepare_application_paths_from(Err(anyhow!("preferred unavailable")), fallback)
        .err()
        .expect("both path failures must abort startup")
        .to_string();

    assert!(error.contains("preferred unavailable"));
    assert!(error.contains("temporary fallback root"));
    assert!(error.contains("blocked-fallback-root"));
}
