use std::{
    error::Error,
    fmt::{self, Display, Formatter},
    sync::{Arc, Condvar, Mutex},
    thread::{self, ThreadId},
};

use anyhow::Result;
use tokio::sync::watch;

use crate::platform::autostart::LaunchAtLogin;
#[cfg(test)]
use crate::platform::autostart::LaunchAtLoginOutcome;

use super::{CompiledConfig, EditableConfig, ValidationError, store::ConfigStore};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigWarning {
    LaunchAtLogin {
        desired: bool,
        confirmed: Option<bool>,
        message: Option<String>,
    },
    LaunchAtLoginRollback {
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
    compiled: Arc<CompiledConfig>,
    warnings: Arc<[ConfigWarning]>,
}

impl PublishedConfig {
    #[cfg(test)]
    pub(crate) fn prepare_for_test(
        editable: EditableConfig,
        revision: u64,
    ) -> Result<Arc<Self>, Vec<ValidationError>> {
        let compiled = CompiledConfig::compile(&editable)?;
        Ok(Arc::new(
            PreparedConfig {
                editable,
                compiled,
                warnings: Vec::new(),
            }
            .publish(revision),
        ))
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn editable(&self) -> &Arc<EditableConfig> {
        &self.editable
    }

    pub fn compiled(&self) -> &Arc<CompiledConfig> {
        &self.compiled
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
            Self::CorrectionSave => "launch-at-login correction save",
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
    UnconfirmedLaunchAtLogin {
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
            | Self::UnconfirmedLaunchAtLogin { warnings }
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
            Self::UnconfirmedLaunchAtLogin { .. } => formatter.write_str(
                "Launch-at-login state could not be confirmed; configuration was not saved or published",
            ),
            Self::Store {
                operation,
                source,
                ..
            } => {
                write!(formatter, "{operation} failed: {source:#}")?;
                if *operation == StoreOperation::CorrectionSave {
                    formatter.write_str(
                        "; the confirmed launch-at-login state was not published and disk was not reported as corrected",
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
    launch_at_login: Arc<dyn LaunchAtLogin>,
    admission: OperationAdmission,
    publications: watch::Sender<Arc<PublishedConfig>>,
}

impl ConfigCoordinator {
    pub fn initial_load(
        store: Arc<ConfigStore>,
        launch_at_login: Arc<dyn LaunchAtLogin>,
    ) -> std::result::Result<Self, ConfigCoordinatorError> {
        Self::initial_load_from(store, launch_at_login)
    }

    #[cfg(test)]
    fn initial_load_with_store(
        store: Arc<dyn CoordinatorStore>,
        launch_at_login: Arc<dyn LaunchAtLogin>,
    ) -> std::result::Result<Self, ConfigCoordinatorError> {
        Self::initial_load_from(store, launch_at_login)
    }

    fn initial_load_from(
        store: Arc<dyn CoordinatorStore>,
        launch_at_login: Arc<dyn LaunchAtLogin>,
    ) -> std::result::Result<Self, ConfigCoordinatorError> {
        let editable = store
            .load()
            .map_err(|source| ConfigCoordinatorError::Store {
                operation: StoreOperation::InitialLoad,
                source,
                warnings: Box::new([]),
            })?;
        let prepared = prepare_loaded(editable, None, store.as_ref(), launch_at_login.as_ref())?;
        let current = Arc::new(prepared.publish(INITIAL_REVISION));
        let (publications, _) = watch::channel(current);
        Ok(Self {
            store,
            launch_at_login,
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

    /// Obtain the exact base for an editor's candidate. Update rechecks the revision
    /// under admission, since another writer may commit while the caller edits it.
    pub fn editable_at_revision(
        &self,
        expected: u64,
    ) -> std::result::Result<EditableConfig, ConfigCoordinatorError> {
        let current = self.current();
        check_revision(expected, current.revision())?;
        Ok(current.editable().as_ref().clone())
    }

    pub fn update(
        &self,
        expected_revision: u64,
        candidate: EditableConfig,
    ) -> std::result::Result<Arc<PublishedConfig>, ConfigCoordinatorError> {
        let _operation = self.admission.enter()?;
        let current = self.current();
        check_revision(expected_revision, current.revision)?;
        let next_revision = current
            .revision
            .checked_add(1)
            .ok_or(ConfigCoordinatorError::RevisionOverflow)?;
        // Settings are not part of the compiled rules, so launch-at-login reconciliation can
        // correct its confirmed boolean without invalidating the prepared rules.
        let compiled = CompiledConfig::compile(&candidate).map_err(|errors| {
            ConfigCoordinatorError::InvalidConfig {
                errors,
                warnings: Box::new([]),
            }
        })?;

        let prepared = prepare_update(
            candidate,
            compiled,
            current.editable.settings.launch_at_login,
            self.store.as_ref(),
            self.launch_at_login.as_ref(),
        )?;
        Ok(self.publish(next_revision, prepared))
    }

    /// Reload publishes only bytes that loaded strictly and, when needed, whose corrected
    /// launch-at-login value was saved. A failed correction is returned with its warning and
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
            Some(current.editable.settings.launch_at_login),
            self.store.as_ref(),
            self.launch_at_login.as_ref(),
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
    compiled: CompiledConfig,
    warnings: Vec<ConfigWarning>,
}

impl PreparedConfig {
    fn publish(self, revision: u64) -> PublishedConfig {
        PublishedConfig {
            revision,
            editable: Arc::new(self.editable),
            compiled: Arc::new(self.compiled),
            warnings: self.warnings.into(),
        }
    }
}

fn prepare_loaded(
    mut editable: EditableConfig,
    previous_launch_at_login: Option<bool>,
    store: &dyn CoordinatorStore,
    launch_at_login: &dyn LaunchAtLogin,
) -> std::result::Result<PreparedConfig, ConfigCoordinatorError> {
    let compiled = CompiledConfig::compile(&editable).map_err(|errors| {
        ConfigCoordinatorError::InvalidConfig {
            errors,
            warnings: Box::new([]),
        }
    })?;
    let desired = editable.settings.launch_at_login;
    let reconciliation = reconcile(launch_at_login, desired);
    let mut warnings = reconciliation.warning.into_iter().collect::<Vec<_>>();
    let Some(confirmed) = reconciliation.confirmed else {
        if let Some(previous) = previous_launch_at_login {
            warnings.push(rollback(launch_at_login, previous));
        }
        return Err(ConfigCoordinatorError::UnconfirmedLaunchAtLogin {
            warnings: warnings.into_boxed_slice(),
        });
    };
    let needs_correction = desired != confirmed;
    editable.settings.launch_at_login = confirmed;

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
        compiled,
        warnings,
    })
}

fn prepare_update(
    mut editable: EditableConfig,
    compiled: CompiledConfig,
    previous_launch_at_login: bool,
    store: &dyn CoordinatorStore,
    launch_at_login: &dyn LaunchAtLogin,
) -> std::result::Result<PreparedConfig, ConfigCoordinatorError> {
    let desired = editable.settings.launch_at_login;
    let reconciliation = reconcile(launch_at_login, desired);
    let mut warnings = reconciliation.warning.into_iter().collect::<Vec<_>>();
    let Some(confirmed) = reconciliation.confirmed else {
        warnings.push(rollback(launch_at_login, previous_launch_at_login));
        return Err(ConfigCoordinatorError::UnconfirmedLaunchAtLogin {
            warnings: warnings.into_boxed_slice(),
        });
    };
    editable.settings.launch_at_login = confirmed;

    if let Err(source) = store.save(&editable) {
        warnings.push(if confirmed == previous_launch_at_login {
            ConfigWarning::LaunchAtLoginRollback {
                target: previous_launch_at_login,
                attempted: false,
                confirmed: Some(confirmed),
                message: None,
            }
        } else {
            rollback(launch_at_login, previous_launch_at_login)
        });
        return Err(ConfigCoordinatorError::Store {
            operation: StoreOperation::Save,
            source,
            warnings: warnings.into_boxed_slice(),
        });
    }

    Ok(PreparedConfig {
        editable,
        compiled,
        warnings,
    })
}

struct Reconciliation {
    confirmed: Option<bool>,
    warning: Option<ConfigWarning>,
}

fn reconcile(launch_at_login: &dyn LaunchAtLogin, desired: bool) -> Reconciliation {
    let outcome = launch_at_login.reconcile(desired);
    let confirmed = outcome.state.confirmed();
    let warning = if confirmed != Some(desired) || outcome.warning.is_some() {
        Some(ConfigWarning::LaunchAtLogin {
            desired,
            confirmed,
            message: outcome.warning,
        })
    } else {
        None
    };
    Reconciliation { confirmed, warning }
}

fn rollback(launch_at_login: &dyn LaunchAtLogin, target: bool) -> ConfigWarning {
    let outcome = launch_at_login.reconcile(target);
    ConfigWarning::LaunchAtLoginRollback {
        target,
        attempted: true,
        confirmed: outcome.state.confirmed(),
        message: outcome.warning,
    }
}

#[cfg(test)]
mod tests;
