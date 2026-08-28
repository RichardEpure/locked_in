#[allow(
    dead_code,
    reason = "UI coordinator migration follows in L-0014 through L-0016"
)]
mod active;
#[allow(
    dead_code,
    reason = "UI coordinator migration follows in L-0014 through L-0016"
)]
mod coordinator;
mod encoding;
mod evaluation;
mod model;
mod paths;
mod store;
mod validation;

use std::{
    path::PathBuf,
    sync::{Arc, OnceLock},
};

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

struct ConfigFacade {
    paths: ApplicationPaths,
    store: Arc<ConfigStore>,
}

static CONFIG_FACADE: OnceLock<ConfigFacade> = OnceLock::new();

pub fn initialize_facade(
    paths: ApplicationPaths,
    store: Arc<ConfigStore>,
) -> std::result::Result<(), &'static str> {
    if store.path() != paths.config_path() {
        return Err("configuration store does not belong to the resolved application paths");
    }
    CONFIG_FACADE
        .set(ConfigFacade { paths, store })
        .map_err(|_| "configuration facade is already initialized")
}

fn facade() -> Result<&'static ConfigFacade> {
    CONFIG_FACADE
        .get()
        .ok_or_else(|| anyhow!("configuration facade is unavailable"))
}

pub fn application_paths() -> Result<&'static ApplicationPaths> {
    Ok(&facade()?.paths)
}

fn config_store() -> Result<&'static ConfigStore> {
    Ok(&facade()?.store)
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
