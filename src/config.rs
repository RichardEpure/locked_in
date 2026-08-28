// Runtime publication will consume this module after the coordinator cutover.
#[allow(dead_code)]
mod active;
#[allow(dead_code)]
mod coordinator;
mod encoding;
mod evaluation;
mod model;
mod paths;
mod store;
mod validation;

use std::{path::PathBuf, sync::LazyLock};

use anyhow::{Result, anyhow};

#[allow(unused_imports)]
pub use active::{ActiveConfig, ActiveDispatch};
#[allow(unused_imports)]
pub use coordinator::{
    ConfigCoordinator, ConfigCoordinatorError, ConfigWarning, PublishedConfig, StartWithWindows,
    StartWithWindowsOutcome, StartWithWindowsState, StoreOperation,
};
#[allow(unused_imports)]
pub use evaluation::EvaluatedAction;
#[allow(unused_imports)]
pub use model::{
    Automation, AutomationCase, Device, EditableConfig, Event, LogLevel, MatchOperator, SendAction,
    Settings, TextCondition, WindowMatcher,
};
pub use paths::{ApplicationPaths, resolve_application_paths};
pub use store::ConfigStore;
#[allow(unused_imports)]
pub use validation::ValidationError;

pub type Config = EditableConfig;

static APPLICATION_PATHS: LazyLock<Result<ApplicationPaths, String>> =
    LazyLock::new(|| resolve_application_paths().map_err(|error| format!("{error:#}")));
static CONFIG_STORE: LazyLock<Result<ConfigStore, String>> = LazyLock::new(|| {
    application_paths()
        .map(|paths| ConfigStore::new(paths.config_path()))
        .map_err(|error| format!("{error:#}"))
});

pub fn application_paths() -> Result<&'static ApplicationPaths> {
    APPLICATION_PATHS
        .as_ref()
        .map_err(|error| anyhow!(error.clone()))
}

fn config_store() -> Result<&'static ConfigStore> {
    CONFIG_STORE
        .as_ref()
        .map_err(|error| anyhow!(error.clone()))
}

pub fn load() -> Result<Config> {
    config_store()?.load()
}

pub fn save(config: &Config) -> Result<()> {
    config_store()?.save(config)
}

pub fn config_path() -> Result<PathBuf> {
    Ok(application_paths()?.config_path())
}

pub fn data_directory() -> Result<PathBuf> {
    Ok(application_paths()?.data_root().to_path_buf())
}
