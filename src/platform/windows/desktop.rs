use std::path::Path;

use anyhow::{Context, Result};

pub(in crate::platform) fn edit_file(path: &Path) -> Result<()> {
    std::process::Command::new("notepad.exe")
        .arg(path)
        .spawn()
        .with_context(|| format!("failed to open {} for editing", path.display()))?;
    Ok(())
}

pub(in crate::platform) fn open_directory(path: &Path) -> Result<()> {
    std::process::Command::new("explorer.exe")
        .arg(path)
        .spawn()
        .with_context(|| format!("failed to open directory {}", path.display()))?;
    Ok(())
}
