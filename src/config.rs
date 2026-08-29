mod active;
mod coordinator;
mod encoding;
#[cfg(test)]
mod evaluation;
mod model;
mod paths;
mod store;
mod validation;

pub use active::ActiveConfig;
#[cfg(test)]
pub use coordinator::StartWithWindowsState;
pub use coordinator::{
    ConfigCoordinator, ConfigCoordinatorError, ConfigWarning, PublishedConfig, StartWithWindows,
    StartWithWindowsOutcome,
};
#[cfg(test)]
pub use model::Event;
pub use model::{
    Automation, AutomationCase, Device, EditableConfig, LogLevel, MatchOperator, SendAction,
    Settings, TextCondition, WindowMatcher,
};
pub use paths::{ApplicationPaths, resolve_application_paths};
pub use store::ConfigStore;
pub use validation::ValidationError;

#[cfg(test)]
pub type Config = EditableConfig;
