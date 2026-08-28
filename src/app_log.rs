use std::{
    fs,
    io::Write,
    path::PathBuf,
    sync::{
        LazyLock, Mutex, OnceLock,
        atomic::{AtomicU8, Ordering},
    },
};

use crate::config::LogLevel;

static LOG_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
static LOG_DIRECTORY: OnceLock<PathBuf> = OnceLock::new();
static LOG_LEVEL: AtomicU8 = AtomicU8::new(1);
const MAX_LOG_SIZE: u64 = 1024 * 1024;
const LOG_COPIES: usize = 5;

pub fn write(message: impl AsRef<str>) {
    write_at(1, message);
}

pub fn write_error(message: impl AsRef<str>) {
    write_at(0, message);
}

pub fn set_level(level: LogLevel) {
    LOG_LEVEL.store(
        match level {
            LogLevel::Error => 0,
            LogLevel::Info => 1,
            LogLevel::Debug => 2,
        },
        Ordering::Relaxed,
    );
}

pub fn initialize(directory: PathBuf) -> Result<(), PathBuf> {
    LOG_DIRECTORY.set(directory)
}

fn write_at(level: u8, message: impl AsRef<str>) {
    if level > LOG_LEVEL.load(Ordering::Relaxed) {
        return;
    }
    let Ok(_guard) = LOG_LOCK.lock() else {
        return;
    };
    let Ok(directory) = log_directory() else {
        return;
    };
    if fs::create_dir_all(&directory).is_err() {
        return;
    }
    let path = directory.join("locked-in.log");
    if path
        .metadata()
        .is_ok_and(|metadata| metadata.len() >= MAX_LOG_SIZE)
    {
        rotate(&directory);
    }
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{timestamp} {}", message.as_ref());
    }
}

pub fn log_directory() -> anyhow::Result<PathBuf> {
    LOG_DIRECTORY
        .get()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("application logging is unavailable"))
}

fn rotate(directory: &std::path::Path) {
    for index in (1..LOG_COPIES).rev() {
        let source = directory.join(format!("locked-in.{}.log", index));
        let destination = directory.join(format!("locked-in.{}.log", index + 1));
        if source.exists() {
            let _ = fs::remove_file(&destination);
            let _ = fs::rename(source, destination);
        }
    }
    let current = directory.join("locked-in.log");
    if current.exists() {
        let destination = directory.join("locked-in.1.log");
        let _ = fs::remove_file(&destination);
        let _ = fs::rename(current, destination);
    }
}
