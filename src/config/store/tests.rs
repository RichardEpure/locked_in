use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Barrier,
        atomic::{AtomicU64, Ordering},
    },
    thread,
};

use super::*;
use crate::config::{ApplicationPaths, Device};

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

struct TestRoot {
    path: PathBuf,
}

impl TestRoot {
    fn new(name: &str) -> Self {
        let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "locked-in-L-0009-{}-{sequence}-{name}",
            std::process::id()
        ));
        if path.exists() {
            fs::remove_dir_all(&path).unwrap();
        }
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn config_path(&self) -> PathBuf {
        self.path.join("config.toml")
    }

    fn temporary_files(&self) -> Vec<PathBuf> {
        fs::read_dir(&self.path)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("config.toml.tmp-"))
            })
            .collect()
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn changed_config() -> EditableConfig {
    let mut config = EditableConfig::default();
    config.settings.start_minimized = false;
    config
}

fn other_config() -> EditableConfig {
    let mut config = EditableConfig::default();
    config.settings.close_to_tray = false;
    config
}

fn paused_store(path: &Path, ready: Arc<Barrier>, release: Arc<Barrier>) -> ConfigStore {
    ConfigStore::with_hooks(
        path,
        TestHooks {
            before_install: Some(Arc::new(move || {
                ready.wait();
                release.wait();
            })),
            ..TestHooks::default()
        },
    )
}

fn assert_rejected_without_rewrite(contents: &str) {
    let root = TestRoot::new("rejected-load");
    let path = root.config_path();
    fs::write(&path, contents).unwrap();
    let before = fs::read(&path).unwrap();

    assert!(ConfigStore::new(&path).load().is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn injected_data_root_derives_every_application_path() {
    let root = TestRoot::new("paths");
    let paths = ApplicationPaths::from_data_root(&root.path);

    assert_eq!(paths.data_root(), root.path);
    assert_eq!(paths.config_path(), root.path.join("config.toml"));
    assert_eq!(paths.log_directory(), root.path.join("logs"));
    assert_eq!(paths.panic_log_path(), root.path.join("panic.log"));
    assert_eq!(paths.webview_data_directory(), root.path.join("webview"));
}

#[test]
fn missing_file_is_created_with_valid_default_config() {
    let root = TestRoot::new("missing");
    let path = root.config_path();
    let store = ConfigStore::new(&path);

    assert_eq!(store.load().unwrap(), EditableConfig::default());
    assert_eq!(store.path(), path);
    assert_eq!(
        encoding::decode(&fs::read_to_string(path).unwrap()).unwrap(),
        EditableConfig::default()
    );
    assert!(root.temporary_files().is_empty());
}

#[test]
fn valid_file_loads_and_reload_reads_the_latest_bytes() {
    let root = TestRoot::new("valid");
    let path = root.config_path();
    fs::write(&path, encoding::encode(&EditableConfig::default()).unwrap()).unwrap();
    let store = ConfigStore::new(path);

    assert_eq!(store.load().unwrap(), EditableConfig::default());
    fs::write(store.path(), encoding::encode(&changed_config()).unwrap()).unwrap();
    assert_eq!(store.reload().unwrap(), changed_config());
}

#[test]
fn malformed_file_is_rejected_without_rewrite() {
    assert_rejected_without_rewrite("this is not valid toml = [");
}

#[test]
fn unsupported_file_is_rejected_without_rewrite() {
    assert_rejected_without_rewrite("version = 3\n");
}

#[test]
fn invalid_file_is_rejected_without_rewrite() {
    let mut invalid = EditableConfig::default();
    invalid.devices.push(Device::default());
    assert_rejected_without_rewrite(&encoding::encode(&invalid).unwrap());
}

#[test]
fn saved_config_round_trips_through_a_new_store() {
    let root = TestRoot::new("round-trip");
    let path = root.config_path();
    ConfigStore::new(&path).save(&changed_config()).unwrap();

    assert_eq!(ConfigStore::new(path).load().unwrap(), changed_config());
}

#[test]
fn save_replaces_existing_file_and_removes_the_owned_temporary_file() {
    let root = TestRoot::new("replacement");
    let path = root.config_path();
    fs::write(&path, "old destination bytes").unwrap();

    ConfigStore::new(&path).save(&changed_config()).unwrap();

    let saved = fs::read_to_string(&path).unwrap();
    assert_eq!(encoding::decode(&saved).unwrap(), changed_config());
    assert!(root.temporary_files().is_empty());
}

#[test]
fn concurrent_stores_only_report_success_after_installing_their_own_bytes() {
    let root = TestRoot::new("concurrent-saves");
    let path = root.config_path();
    fs::write(&path, encoding::encode(&EditableConfig::default()).unwrap()).unwrap();

    let a_ready = Arc::new(Barrier::new(2));
    let a_release = Arc::new(Barrier::new(2));
    let b_ready = Arc::new(Barrier::new(2));
    let b_release = Arc::new(Barrier::new(2));
    let store_a = paused_store(&path, Arc::clone(&a_ready), Arc::clone(&a_release));
    let store_b = paused_store(&path, Arc::clone(&b_ready), Arc::clone(&b_release));
    let config_a = changed_config();
    let config_b = other_config();

    let thread_a = thread::spawn({
        let config = config_a.clone();
        move || store_a.save(&config)
    });
    a_ready.wait();
    let thread_b = thread::spawn({
        let config = config_b.clone();
        move || store_b.save(&config)
    });
    b_ready.wait();

    a_release.wait();
    let result_a = thread_a.join().unwrap();
    let installed_by_a = ConfigStore::new(&path).load();
    b_release.wait();
    let result_b = thread_b.join().unwrap();
    let installed_by_b = ConfigStore::new(&path).load();

    assert!(result_a.is_ok());
    assert_eq!(installed_by_a.unwrap(), config_a);
    assert!(result_b.is_ok());
    assert_eq!(installed_by_b.unwrap(), config_b);
    assert!(root.temporary_files().is_empty());
}

#[test]
fn missing_initialization_reads_a_file_installed_by_another_store() {
    let root = TestRoot::new("concurrent-initialization");
    let path = root.config_path();
    let ready = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let initializing_store = paused_store(&path, Arc::clone(&ready), Arc::clone(&release));
    let winner = changed_config();

    let initializing_thread = thread::spawn(move || initializing_store.load());
    ready.wait();
    ConfigStore::new(&path).save(&winner).unwrap();
    release.wait();
    let loaded = initializing_thread.join().unwrap().unwrap();

    assert_eq!(loaded, winner);
    assert_eq!(ConfigStore::new(&path).load().unwrap(), winner);
    assert!(root.temporary_files().is_empty());
}

#[test]
fn failed_validation_leaves_destination_bytes_unchanged() {
    let root = TestRoot::new("validation-failure");
    let path = root.config_path();
    let original = encoding::encode(&EditableConfig::default()).unwrap();
    fs::write(&path, &original).unwrap();
    let mut invalid = EditableConfig::default();
    invalid.devices.push(Device::default());

    assert!(ConfigStore::new(&path).save(&invalid).is_err());
    assert_eq!(fs::read_to_string(path).unwrap(), original);
    assert!(root.temporary_files().is_empty());
}

#[test]
fn failed_temporary_write_leaves_destination_unchanged_and_cleans_up() {
    let root = TestRoot::new("write-failure");
    let path = root.config_path();
    let original = encoding::encode(&EditableConfig::default()).unwrap();
    fs::write(&path, &original).unwrap();
    let store = ConfigStore::with_hooks(
        &path,
        TestHooks {
            fail_before_write: true,
            ..TestHooks::default()
        },
    );

    assert!(store.save(&changed_config()).is_err());
    assert_eq!(fs::read_to_string(path).unwrap(), original);
    assert!(root.temporary_files().is_empty());
}

#[cfg(windows)]
#[test]
fn failed_atomic_replace_leaves_destination_bytes_unchanged_and_cleans_up() {
    use std::{fs::OpenOptions, os::windows::fs::OpenOptionsExt};

    let root = TestRoot::new("replace-failure");
    let path = root.config_path();
    let original = encoding::encode(&EditableConfig::default()).unwrap();
    fs::write(&path, &original).unwrap();
    let locked_destination = OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&path)
        .unwrap();

    assert!(ConfigStore::new(&path).save(&changed_config()).is_err());
    drop(locked_destination);
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    assert!(root.temporary_files().is_empty());
}
