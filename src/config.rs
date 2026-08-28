mod encoding;
mod evaluation;
mod model;
mod validation;

use std::{env, fs, io::Write, path::PathBuf};

use anyhow::{Context, Result};

#[allow(unused_imports)]
pub use evaluation::EvaluatedAction;
#[allow(unused_imports)]
pub use model::{
    Automation, AutomationCase, Device, EditableConfig, Event, LogLevel, MatchOperator, SendAction,
    Settings, TextCondition, WindowMatcher,
};
#[allow(unused_imports)]
pub use validation::ValidationError;

pub type Config = EditableConfig;

const CONFIG_PATH: &str = "config.toml";

pub fn load() -> Result<Config> {
    let path = config_path()?;
    if !path.is_file() {
        let config = Config::default();
        save(&config)?;
        return Ok(config);
    }

    let contents =
        fs::read_to_string(&path).with_context(|| format!("Failed to load {}", path.display()))?;
    let config = encoding::decode(&contents)
        .with_context(|| format!("Failed to load {}", path.display()))?;

    let errors = config.validate();
    if !errors.is_empty() {
        anyhow::bail!(validation::format_errors(&errors));
    }
    Ok(config)
}

pub fn save(config: &Config) -> Result<()> {
    let errors = config.validate();
    if !errors.is_empty() {
        anyhow::bail!(validation::format_errors(&errors));
    }

    let path = config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }

    let contents = encoding::encode(config).context("Failed to serialize config")?;
    let temporary = path.with_extension("toml.tmp");
    let mut file = fs::File::create(&temporary)
        .with_context(|| format!("Failed to create {}", temporary.display()))?;
    file.write_all(contents.as_bytes())
        .context("Failed to write temporary config")?;
    file.flush().context("Failed to flush temporary config")?;
    replace_file(&temporary, &path)?;
    Ok(())
}

pub fn config_path() -> Result<PathBuf> {
    Ok(data_directory()?.join(CONFIG_PATH))
}

pub fn data_directory() -> Result<PathBuf> {
    if let Some(path) = env::var_os("LOCKED_IN_DATA_DIR") {
        return Ok(PathBuf::from(path));
    }
    if cfg!(debug_assertions) {
        return env::current_dir().context("Failed to get current directory");
    }
    env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|path| path.join("LockedIn"))
        .context("LOCALAPPDATA is unavailable")
}

#[cfg(windows)]
fn replace_file(source: &std::path::Path, destination: &std::path::Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::Storage::FileSystem::{
            MOVE_FILE_FLAGS, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        },
        core::PCWSTR,
    };

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
        MoveFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(destination.as_ptr()),
            MOVE_FILE_FLAGS(MOVEFILE_REPLACE_EXISTING.0 | MOVEFILE_WRITE_THROUGH.0),
        )
        .with_context(|| "Failed to atomically replace configuration")?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(source: &std::path::Path, destination: &std::path::Path) -> Result<()> {
    fs::rename(source, destination)
        .with_context(|| format!("Failed to replace {}", destination.display()))
}
