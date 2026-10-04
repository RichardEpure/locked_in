use std::path::PathBuf;

use anyhow::{Context, Result};

pub(in crate::platform) fn default_data_root() -> Result<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|path| path.join("LockedIn"))
        .context("LOCALAPPDATA is unavailable")
}
