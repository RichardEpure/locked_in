mod compiled;
mod coordinator;
mod encoding;
mod model;
mod paths;
mod store;
mod validation;

pub use compiled::CompiledConfig;
#[cfg(test)]
pub use coordinator::StartWithWindowsState;
pub use coordinator::{
    ConfigCoordinator, ConfigCoordinatorError, ConfigWarning, PublishedConfig, StartWithWindows,
    StartWithWindowsOutcome,
};
pub use model::{
    Automation, AutomationCase, Device, EditableConfig, EventKind, LogLevel, MatchOperator,
    SendAction, Settings, TextCondition, WindowMatcher,
};
pub use paths::{ApplicationPaths, resolve_application_paths};
pub use store::ConfigStore;
pub use validation::ValidationError;
