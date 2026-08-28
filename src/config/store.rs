use std::{
    fs::{self, File, OpenOptions},
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use anyhow::{Context, Result, anyhow, bail};

use super::{EditableConfig, encoding, validation};

static NEXT_TEMPORARY_FILE: AtomicU64 = AtomicU64::new(0);

pub struct ConfigStore {
    path: PathBuf,
    access: Mutex<()>,
    #[cfg(test)]
    hooks: TestHooks,
}

impl ConfigStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            access: Mutex::new(()),
            #[cfg(test)]
            hooks: TestHooks::default(),
        }
    }

    #[cfg(test)]
    fn with_hooks(path: impl Into<PathBuf>, hooks: TestHooks) -> Self {
        Self {
            path: path.into(),
            access: Mutex::new(()),
            hooks,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<EditableConfig> {
        let _guard = self.lock()?;
        match fs::read_to_string(&self.path) {
            Ok(contents) => self.decode_and_validate(&contents),
            Err(error) if error.kind() == ErrorKind::NotFound => self.initialize_missing(),
            Err(error) => {
                Err(error).with_context(|| format!("Failed to load {}", self.path.display()))
            }
        }
    }

    #[allow(dead_code)]
    pub fn reload(&self) -> Result<EditableConfig> {
        self.load()
    }

    pub fn save(&self, config: &EditableConfig) -> Result<()> {
        let _guard = self.lock()?;
        let temporary = self.prepare_temporary(config)?;
        self.before_install();
        replace_file(temporary.path(), &self.path)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, ()>> {
        self.access
            .lock()
            .map_err(|_| anyhow!("Configuration store lock is poisoned"))
    }

    fn decode_and_validate(&self, contents: &str) -> Result<EditableConfig> {
        let config = encoding::decode(contents)
            .with_context(|| format!("Failed to load {}", self.path.display()))?;
        validate(&config)?;
        Ok(config)
    }

    fn initialize_missing(&self) -> Result<EditableConfig> {
        let default = EditableConfig::default();
        let temporary = self.prepare_temporary(&default)?;
        self.before_install();
        let outcome = install_file_if_absent(temporary.path(), &self.path)?;
        drop(temporary);

        match outcome {
            InstallOutcome::Installed => Ok(default),
            InstallOutcome::DestinationExists => {
                let contents = fs::read_to_string(&self.path)
                    .with_context(|| format!("Failed to load {}", self.path.display()))?;
                self.decode_and_validate(&contents)
            }
        }
    }

    fn prepare_temporary(&self, config: &EditableConfig) -> Result<TemporaryFile> {
        validate(config)?;
        let contents = encoding::encode(config).context("Failed to serialize config")?;
        let parent = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .context("Configuration path has no parent directory")?;
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;

        let mut temporary = TemporaryFile::create(&self.path)?;
        #[cfg(test)]
        if self.hooks.fail_before_write {
            bail!("Injected temporary write failure");
        }
        temporary.write_and_sync(&contents)?;
        Ok(temporary)
    }

    fn before_install(&self) {
        #[cfg(test)]
        if let Some(hook) = &self.hooks.before_install {
            hook();
        }
    }
}

fn validate(config: &EditableConfig) -> Result<()> {
    let errors = config.validate();
    if !errors.is_empty() {
        bail!(validation::format_errors(&errors));
    }
    Ok(())
}

struct TemporaryFile {
    path: PathBuf,
    file: Option<File>,
}

impl TemporaryFile {
    fn create(destination: &Path) -> Result<Self> {
        let file_name = destination
            .file_name()
            .context("Configuration path has no file name")?;
        loop {
            let sequence = NEXT_TEMPORARY_FILE.fetch_add(1, Ordering::Relaxed);
            let mut temporary_name = file_name.to_os_string();
            temporary_name.push(format!(".tmp-{}-{sequence}", std::process::id()));
            let path = destination.with_file_name(temporary_name);
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => {
                    return Ok(Self {
                        path,
                        file: Some(file),
                    });
                }
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("Failed to create {}", path.display()));
                }
            }
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn write_and_sync(&mut self, contents: &str) -> Result<()> {
        let file = self.file.as_mut().expect("temporary file is still open");
        file.write_all(contents.as_bytes())
            .context("Failed to write temporary config")?;
        file.sync_all()
            .context("Failed to synchronize temporary config")?;
        self.file.take();
        Ok(())
    }
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        self.file.take();
        let _ = fs::remove_file(&self.path);
    }
}

enum InstallOutcome {
    Installed,
    DestinationExists,
}

#[cfg(windows)]
fn install_file_if_absent(source: &Path, destination: &Path) -> Result<InstallOutcome> {
    match move_file(source, destination, MOVEFILE_WRITE_THROUGH) {
        Ok(()) => Ok(InstallOutcome::Installed),
        Err(error) if matches!(error.raw_os_error(), Some(80 | 183)) => {
            Ok(InstallOutcome::DestinationExists)
        }
        Err(error) => Err(error).context("Failed to install initial configuration"),
    }
}

#[cfg(not(windows))]
fn install_file_if_absent(source: &Path, destination: &Path) -> Result<InstallOutcome> {
    match fs::hard_link(source, destination) {
        Ok(()) => Ok(InstallOutcome::Installed),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            Ok(InstallOutcome::DestinationExists)
        }
        Err(error) => Err(error).context("Failed to install initial configuration"),
    }
}

#[cfg(windows)]
use windows::Win32::Storage::FileSystem::{
    MOVE_FILE_FLAGS, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> Result<()> {
    move_file(
        source,
        destination,
        MOVE_FILE_FLAGS(MOVEFILE_REPLACE_EXISTING.0 | MOVEFILE_WRITE_THROUGH.0),
    )
    .context("Failed to atomically replace configuration")
}

#[cfg(windows)]
fn move_file(source: &Path, destination: &Path, flags: MOVE_FILE_FLAGS) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{Win32::Storage::FileSystem::MoveFileExW, core::PCWSTR};

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    unsafe {
        MoveFileExW(PCWSTR(source.as_ptr()), PCWSTR(destination.as_ptr()), flags)
            .map_err(|_| std::io::Error::last_os_error())
    }
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> Result<()> {
    fs::rename(source, destination)
        .with_context(|| format!("Failed to replace {}", destination.display()))
}

#[cfg(test)]
#[derive(Default)]
struct TestHooks {
    fail_before_write: bool,
    before_install: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
}

#[cfg(test)]
mod tests;
