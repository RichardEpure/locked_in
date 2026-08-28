use std::{
    env,
    path::{self, Path, PathBuf},
};

use anyhow::{Context, Result};

const CONFIG_FILE: &str = "config.toml";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationPaths {
    data_root: PathBuf,
}

impl ApplicationPaths {
    pub fn from_data_root(data_root: impl Into<PathBuf>) -> Self {
        Self {
            data_root: data_root.into(),
        }
    }

    pub fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub fn config_path(&self) -> PathBuf {
        self.data_root.join(CONFIG_FILE)
    }

    #[allow(dead_code)]
    pub fn log_directory(&self) -> PathBuf {
        self.data_root.join("logs")
    }

    #[allow(dead_code)]
    pub fn panic_log_path(&self) -> PathBuf {
        self.data_root.join("panic.log")
    }

    #[allow(dead_code)]
    pub fn webview_data_directory(&self) -> PathBuf {
        self.data_root.join("webview")
    }
}

pub fn resolve_application_paths() -> Result<ApplicationPaths> {
    if let Some(path) = env::var_os("LOCKED_IN_DATA_DIR") {
        return resolve_override(PathBuf::from(path));
    }
    if cfg!(debug_assertions) {
        return env::current_dir()
            .map(ApplicationPaths::from_data_root)
            .context("Failed to get current directory");
    }
    env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|path| ApplicationPaths::from_data_root(path.join("LockedIn")))
        .context("LOCALAPPDATA is unavailable")
}

fn resolve_override(data_root: PathBuf) -> Result<ApplicationPaths> {
    path::absolute(data_root)
        .map(ApplicationPaths::from_data_root)
        .context("Failed to resolve LOCKED_IN_DATA_DIR")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_override_is_resolved_to_an_absolute_startup_path() {
        let paths = resolve_override(PathBuf::from("relative-data-root")).unwrap();

        assert!(paths.data_root().is_absolute());
        assert!(paths.data_root().ends_with("relative-data-root"));
    }

    #[test]
    fn every_artifact_path_has_the_same_data_root_identity() {
        let paths = ApplicationPaths::from_data_root(PathBuf::from("one-root"));

        assert_eq!(paths.config_path().parent(), Some(paths.data_root()));
        assert_eq!(paths.log_directory().parent(), Some(paths.data_root()));
        assert_eq!(paths.panic_log_path().parent(), Some(paths.data_root()));
        assert_eq!(
            paths.webview_data_directory().parent(),
            Some(paths.data_root())
        );
    }
}
