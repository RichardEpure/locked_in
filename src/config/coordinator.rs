use std::{
    error::Error,
    fmt::{self, Display, Formatter},
    sync::{Arc, Condvar, Mutex},
    thread::{self, ThreadId},
};

use anyhow::Result;
use tokio::sync::watch;

use super::{ActiveConfig, EditableConfig, ValidationError, store::ConfigStore};

const INITIAL_REVISION: u64 = 1;

// Store calls run under operation admission. Implementations must not synchronously wait for a
// different thread to start another operation on the same coordinator.
trait CoordinatorStore: Send + Sync {
    fn load(&self) -> Result<EditableConfig>;
    fn save(&self, config: &EditableConfig) -> Result<()>;
}

impl CoordinatorStore for ConfigStore {
    fn load(&self) -> Result<EditableConfig> {
        ConfigStore::load(self)
    }

    fn save(&self, config: &EditableConfig) -> Result<()> {
        ConfigStore::save(self, config)
    }
}

/// Applies and confirms the Windows startup setting synchronously.
///
/// `current` and `subscribe` may be called from an implementation. Same-thread mutation reentry
/// is rejected. Implementations must not wait for another thread to call `update` or `reload` on
/// the same coordinator because durable operations are intentionally serialized.
pub trait StartWithWindows: Send + Sync {
    fn reconcile(&self, desired: bool) -> StartWithWindowsOutcome;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartWithWindowsState {
    Confirmed(bool),
    Unconfirmed,
}

impl StartWithWindowsState {
    fn confirmed(self) -> Option<bool> {
        match self {
            Self::Confirmed(confirmed) => Some(confirmed),
            Self::Unconfirmed => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartWithWindowsOutcome {
    pub state: StartWithWindowsState,
    pub warning: Option<String>,
}

impl StartWithWindowsOutcome {
    pub fn confirmed(confirmed: bool) -> Self {
        Self {
            state: StartWithWindowsState::Confirmed(confirmed),
            warning: None,
        }
    }

    pub fn warning(confirmed: bool, warning: impl Into<String>) -> Self {
        Self {
            state: StartWithWindowsState::Confirmed(confirmed),
            warning: Some(warning.into()),
        }
    }

    pub fn unconfirmed(warning: impl Into<String>) -> Self {
        Self {
            state: StartWithWindowsState::Unconfirmed,
            warning: Some(warning.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigWarning {
    StartWithWindows {
        desired: bool,
        confirmed: Option<bool>,
        message: Option<String>,
    },
    StartWithWindowsRollback {
        target: bool,
        attempted: bool,
        confirmed: Option<bool>,
        message: Option<String>,
    },
}

#[derive(Debug)]
pub struct PublishedConfig {
    revision: u64,
    editable: Arc<EditableConfig>,
    active: Arc<ActiveConfig>,
    warnings: Arc<[ConfigWarning]>,
}

impl PublishedConfig {
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn editable(&self) -> &Arc<EditableConfig> {
        &self.editable
    }

    pub fn active(&self) -> &Arc<ActiveConfig> {
        &self.active
    }

    pub fn warnings(&self) -> &[ConfigWarning] {
        &self.warnings
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreOperation {
    InitialLoad,
    Reload,
    Save,
    CorrectionSave,
}

impl Display for StoreOperation {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InitialLoad => "initial configuration load",
            Self::Reload => "configuration reload",
            Self::Save => "configuration save",
            Self::CorrectionSave => "Start with Windows correction save",
        })
    }
}

#[derive(Debug)]
pub enum ConfigCoordinatorError {
    StaleRevision {
        expected: u64,
        actual: u64,
    },
    ReentrantOperation,
    RevisionOverflow,
    InvalidConfig {
        errors: Vec<ValidationError>,
        warnings: Box<[ConfigWarning]>,
    },
    UnconfirmedStartWithWindows {
        warnings: Box<[ConfigWarning]>,
    },
    Store {
        operation: StoreOperation,
        source: anyhow::Error,
        warnings: Box<[ConfigWarning]>,
    },
}

impl ConfigCoordinatorError {
    pub fn warnings(&self) -> &[ConfigWarning] {
        match self {
            Self::InvalidConfig { warnings, .. }
            | Self::UnconfirmedStartWithWindows { warnings }
            | Self::Store { warnings, .. } => warnings,
            Self::StaleRevision { .. } | Self::ReentrantOperation | Self::RevisionOverflow => &[],
        }
    }
}

impl Display for ConfigCoordinatorError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleRevision { expected, actual } => write!(
                formatter,
                "stale configuration revision {expected}; current revision is {actual}"
            ),
            Self::ReentrantOperation => formatter.write_str(
                "configuration operation rejected because an adapter or store reentered the coordinator",
            ),
            Self::RevisionOverflow => formatter.write_str("configuration revision overflow"),
            Self::InvalidConfig { errors, .. } => {
                formatter.write_str("configuration validation or compilation failed")?;
                for error in errors {
                    write!(formatter, "\n{}: {}", error.path, error.message)?;
                }
                Ok(())
            }
            Self::UnconfirmedStartWithWindows { .. } => formatter.write_str(
                "Start with Windows state could not be confirmed; configuration was not saved or published",
            ),
            Self::Store {
                operation,
                source,
                ..
            } => {
                write!(formatter, "{operation} failed: {source:#}")?;
                if *operation == StoreOperation::CorrectionSave {
                    formatter.write_str(
                        "; the confirmed Start with Windows state was not published and disk was not reported as corrected",
                    )?;
                }
                Ok(())
            }
        }
    }
}

impl Error for ConfigCoordinatorError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Store { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}

#[derive(Default)]
struct AdmissionState {
    owner: Option<ThreadId>,
}

#[derive(Default)]
struct OperationAdmission {
    state: Mutex<AdmissionState>,
    available: Condvar,
}

impl OperationAdmission {
    fn reject_reentrant(&self) -> std::result::Result<(), ConfigCoordinatorError> {
        let state = self.lock_state();
        if state.owner == Some(thread::current().id()) {
            return Err(ConfigCoordinatorError::ReentrantOperation);
        }
        Ok(())
    }

    fn enter(&self) -> std::result::Result<OperationGuard<'_>, ConfigCoordinatorError> {
        let owner = thread::current().id();
        let mut state = self.lock_state();
        loop {
            match state.owner {
                None => {
                    state.owner = Some(owner);
                    return Ok(OperationGuard { admission: self });
                }
                Some(current) if current == owner => {
                    return Err(ConfigCoordinatorError::ReentrantOperation);
                }
                Some(_) => {
                    state = self
                        .available
                        .wait(state)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                }
            }
        }
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, AdmissionState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

struct OperationGuard<'a> {
    admission: &'a OperationAdmission,
}

impl Drop for OperationGuard<'_> {
    fn drop(&mut self) {
        let mut state = self.admission.lock_state();
        state.owner = None;
        self.admission.available.notify_one();
    }
}

pub struct ConfigCoordinator {
    store: Arc<dyn CoordinatorStore>,
    start_with_windows: Arc<dyn StartWithWindows>,
    admission: OperationAdmission,
    publications: watch::Sender<Arc<PublishedConfig>>,
}

impl ConfigCoordinator {
    pub fn initial_load(
        store: ConfigStore,
        start_with_windows: Arc<dyn StartWithWindows>,
    ) -> std::result::Result<Self, ConfigCoordinatorError> {
        Self::initial_load_from(Arc::new(store), start_with_windows)
    }

    #[cfg(test)]
    fn initial_load_with_store(
        store: Arc<dyn CoordinatorStore>,
        start_with_windows: Arc<dyn StartWithWindows>,
    ) -> std::result::Result<Self, ConfigCoordinatorError> {
        Self::initial_load_from(store, start_with_windows)
    }

    fn initial_load_from(
        store: Arc<dyn CoordinatorStore>,
        start_with_windows: Arc<dyn StartWithWindows>,
    ) -> std::result::Result<Self, ConfigCoordinatorError> {
        let editable = store
            .load()
            .map_err(|source| ConfigCoordinatorError::Store {
                operation: StoreOperation::InitialLoad,
                source,
                warnings: Box::new([]),
            })?;
        let prepared = prepare_loaded(editable, None, store.as_ref(), start_with_windows.as_ref())?;
        let current = Arc::new(prepared.publish(INITIAL_REVISION));
        let (publications, _) = watch::channel(current);
        Ok(Self {
            store,
            start_with_windows,
            admission: OperationAdmission::default(),
            publications,
        })
    }

    pub fn current(&self) -> Arc<PublishedConfig> {
        self.publications.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<Arc<PublishedConfig>> {
        self.publications.subscribe()
    }

    pub fn update(
        &self,
        expected_revision: u64,
        build: impl FnOnce(&EditableConfig) -> EditableConfig,
    ) -> std::result::Result<Arc<PublishedConfig>, ConfigCoordinatorError> {
        self.admission.reject_reentrant()?;
        let base = self.current();
        check_revision(expected_revision, base.revision)?;

        let candidate = build(&base.editable);
        // ActiveConfig compiles automations and devices only, so the later confirmed startup
        // setting can replace the requested boolean without invalidating this snapshot.
        let compiled = ActiveConfig::compile(&candidate);

        let _operation = self.admission.enter()?;
        let current = self.current();
        check_revision(expected_revision, current.revision)?;
        let next_revision = current
            .revision
            .checked_add(1)
            .ok_or(ConfigCoordinatorError::RevisionOverflow)?;
        let active = compiled.map_err(|errors| ConfigCoordinatorError::InvalidConfig {
            errors,
            warnings: Box::new([]),
        })?;

        let prepared = prepare_update(
            candidate,
            active,
            current.editable.settings.start_with_windows,
            self.store.as_ref(),
            self.start_with_windows.as_ref(),
        )?;
        Ok(self.publish(next_revision, prepared))
    }

    /// Reload publishes only bytes that loaded strictly and, when needed, whose corrected
    /// Start-with-Windows value was saved. A failed correction is returned with its warning and
    /// leaves the previous publication in place; it does not claim that disk matches the OS.
    pub fn reload(&self) -> std::result::Result<Arc<PublishedConfig>, ConfigCoordinatorError> {
        let _operation = self.admission.enter()?;
        let current = self.current();
        let next_revision = current
            .revision
            .checked_add(1)
            .ok_or(ConfigCoordinatorError::RevisionOverflow)?;
        let editable = self
            .store
            .load()
            .map_err(|source| ConfigCoordinatorError::Store {
                operation: StoreOperation::Reload,
                source,
                warnings: Box::new([]),
            })?;
        let prepared = prepare_loaded(
            editable,
            Some(current.editable.settings.start_with_windows),
            self.store.as_ref(),
            self.start_with_windows.as_ref(),
        )?;
        Ok(self.publish(next_revision, prepared))
    }

    fn publish(&self, revision: u64, prepared: PreparedConfig) -> Arc<PublishedConfig> {
        let publication = Arc::new(prepared.publish(revision));
        self.publications.send_replace(Arc::clone(&publication));
        publication
    }
}

fn check_revision(expected: u64, actual: u64) -> std::result::Result<(), ConfigCoordinatorError> {
    if expected != actual {
        return Err(ConfigCoordinatorError::StaleRevision { expected, actual });
    }
    Ok(())
}

struct PreparedConfig {
    editable: EditableConfig,
    active: ActiveConfig,
    warnings: Vec<ConfigWarning>,
}

impl PreparedConfig {
    fn publish(self, revision: u64) -> PublishedConfig {
        PublishedConfig {
            revision,
            editable: Arc::new(self.editable),
            active: Arc::new(self.active),
            warnings: self.warnings.into(),
        }
    }
}

fn prepare_loaded(
    mut editable: EditableConfig,
    previous_start_with_windows: Option<bool>,
    store: &dyn CoordinatorStore,
    start_with_windows: &dyn StartWithWindows,
) -> std::result::Result<PreparedConfig, ConfigCoordinatorError> {
    let active = ActiveConfig::compile(&editable).map_err(|errors| {
        ConfigCoordinatorError::InvalidConfig {
            errors,
            warnings: Box::new([]),
        }
    })?;
    let desired = editable.settings.start_with_windows;
    let reconciliation = reconcile(start_with_windows, desired);
    let mut warnings = reconciliation.warning.into_iter().collect::<Vec<_>>();
    let Some(confirmed) = reconciliation.confirmed else {
        if let Some(previous) = previous_start_with_windows {
            warnings.push(rollback(start_with_windows, previous));
        }
        return Err(ConfigCoordinatorError::UnconfirmedStartWithWindows {
            warnings: warnings.into_boxed_slice(),
        });
    };
    let needs_correction = desired != confirmed;
    editable.settings.start_with_windows = confirmed;

    if needs_correction {
        store
            .save(&editable)
            .map_err(|source| ConfigCoordinatorError::Store {
                operation: StoreOperation::CorrectionSave,
                source,
                warnings: warnings.clone().into_boxed_slice(),
            })?;
    }

    Ok(PreparedConfig {
        editable,
        active,
        warnings,
    })
}

fn prepare_update(
    mut editable: EditableConfig,
    active: ActiveConfig,
    previous_start_with_windows: bool,
    store: &dyn CoordinatorStore,
    start_with_windows: &dyn StartWithWindows,
) -> std::result::Result<PreparedConfig, ConfigCoordinatorError> {
    let desired = editable.settings.start_with_windows;
    let reconciliation = reconcile(start_with_windows, desired);
    let mut warnings = reconciliation.warning.into_iter().collect::<Vec<_>>();
    let Some(confirmed) = reconciliation.confirmed else {
        warnings.push(rollback(start_with_windows, previous_start_with_windows));
        return Err(ConfigCoordinatorError::UnconfirmedStartWithWindows {
            warnings: warnings.into_boxed_slice(),
        });
    };
    editable.settings.start_with_windows = confirmed;

    if let Err(source) = store.save(&editable) {
        warnings.push(if confirmed == previous_start_with_windows {
            ConfigWarning::StartWithWindowsRollback {
                target: previous_start_with_windows,
                attempted: false,
                confirmed: Some(confirmed),
                message: None,
            }
        } else {
            rollback(start_with_windows, previous_start_with_windows)
        });
        return Err(ConfigCoordinatorError::Store {
            operation: StoreOperation::Save,
            source,
            warnings: warnings.into_boxed_slice(),
        });
    }

    Ok(PreparedConfig {
        editable,
        active,
        warnings,
    })
}

struct Reconciliation {
    confirmed: Option<bool>,
    warning: Option<ConfigWarning>,
}

fn reconcile(start_with_windows: &dyn StartWithWindows, desired: bool) -> Reconciliation {
    let outcome = start_with_windows.reconcile(desired);
    let confirmed = outcome.state.confirmed();
    let warning = if confirmed != Some(desired) || outcome.warning.is_some() {
        Some(ConfigWarning::StartWithWindows {
            desired,
            confirmed,
            message: outcome.warning,
        })
    } else {
        None
    };
    Reconciliation { confirmed, warning }
}

fn rollback(start_with_windows: &dyn StartWithWindows, target: bool) -> ConfigWarning {
    let outcome = start_with_windows.reconcile(target);
    ConfigWarning::StartWithWindowsRollback {
        target,
        attempted: true,
        confirmed: outcome.state.confirmed(),
        message: outcome.warning,
    }
}

#[cfg(test)]
mod tests;
