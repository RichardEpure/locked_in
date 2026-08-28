mod worker;

use std::{
    error::Error,
    fmt::{self, Display, Formatter},
    sync::{Arc, Mutex, RwLock},
    time::Duration,
};

use anyhow::Result;
use tokio::sync::{mpsc, oneshot, watch};

use crate::{
    config::{Config, Device, SendAction},
    hid::{HidBackend, HidInventory},
    win::WindowMetadata,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FocusSourceState {
    Available,
    Unavailable(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimePhase {
    Starting,
    Active,
    Degraded,
    Unavailable,
    Stopping,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeStatus {
    pub phase: RuntimePhase,
    pub detail: Option<String>,
}

impl RuntimeStatus {
    fn starting() -> Self {
        Self {
            phase: RuntimePhase::Starting,
            detail: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TestDispatchResult {
    pub sent: usize,
    pub failures: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeLifecycle {
    Starting,
    Running,
    Stopping,
    Stopped,
}

struct RuntimeHealth {
    focus_error: Option<String>,
    refresh_error: Option<String>,
    dispatch_error: Option<String>,
    worker_error: Option<String>,
    has_config: bool,
    lifecycle: RuntimeLifecycle,
}

impl RuntimeHealth {
    fn status(&self) -> RuntimeStatus {
        match self.lifecycle {
            RuntimeLifecycle::Starting => return RuntimeStatus::starting(),
            RuntimeLifecycle::Stopping => {
                return RuntimeStatus {
                    phase: RuntimePhase::Stopping,
                    detail: None,
                };
            }
            RuntimeLifecycle::Stopped => {
                return RuntimeStatus {
                    phase: RuntimePhase::Stopped,
                    detail: None,
                };
            }
            RuntimeLifecycle::Running => {}
        }

        let mut unavailable = Vec::new();
        if let Some(error) = &self.worker_error {
            unavailable.push(error.clone());
        }
        if let Some(error) = &self.focus_error {
            unavailable.push(error.clone());
        }
        if !self.has_config {
            unavailable.push("configuration is unavailable".to_string());
        }
        if !unavailable.is_empty() {
            return RuntimeStatus {
                phase: RuntimePhase::Unavailable,
                detail: Some(unavailable.join("; ")),
            };
        }
        let degraded = self
            .refresh_error
            .iter()
            .chain(&self.dispatch_error)
            .cloned()
            .collect::<Vec<_>>();
        if !degraded.is_empty() {
            return RuntimeStatus {
                phase: RuntimePhase::Degraded,
                detail: Some(degraded.join("; ")),
            };
        }
        RuntimeStatus {
            phase: RuntimePhase::Active,
            detail: None,
        }
    }
}

struct Admission {
    refresh_pending: bool,
    shutdown_requested: bool,
}

struct Shared {
    config: RwLock<Option<Arc<Config>>>,
    health: Mutex<RuntimeHealth>,
    status: watch::Sender<RuntimeStatus>,
    #[cfg(test)]
    status_history: Mutex<Vec<RuntimeStatus>>,
    hid_inventory: watch::Sender<Arc<HidInventory>>,
    commands: mpsc::Sender<RuntimeCommand>,
    shutdown: watch::Sender<bool>,
    admission: Mutex<Admission>,
}

enum RuntimeCommand {
    TestAction {
        action: SendAction,
        devices: Vec<Device>,
        response: oneshot::Sender<std::result::Result<TestDispatchResult, RuntimeRequestError>>,
    },
    RefreshHid,
}

const ORDINARY_COMMAND_CAPACITY: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HidRefreshRequestResult {
    Queued,
    AlreadyPending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeRequestError {
    Busy,
    Unavailable,
    Cancelled,
}

impl Display for RuntimeRequestError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Busy => "automation runtime command queue is full",
            Self::Unavailable => "automation runtime is stopping or unavailable",
            Self::Cancelled => "automation runtime cancelled the command before execution",
        })
    }
}

impl Error for RuntimeRequestError {}

#[derive(Clone)]
pub(crate) struct AutomationRuntime {
    shared: Arc<Shared>,
}

impl AutomationRuntime {
    pub fn start(
        initial_config: Option<Config>,
        focus_events: watch::Receiver<WindowMetadata>,
        focus_source: FocusSourceState,
        backend: impl HidBackend,
    ) -> Result<(Self, RuntimeOwner)> {
        let (commands, command_rx) = mpsc::channel(ORDINARY_COMMAND_CAPACITY);
        let (shutdown, shutdown_rx) = watch::channel(false);
        let (status, _) = watch::channel(RuntimeStatus::starting());
        let (hid_inventory, _) = watch::channel(Arc::new(HidInventory::default()));
        let (completed, completion_rx) = std::sync::mpsc::channel();
        let focus_error = match focus_source {
            FocusSourceState::Available => None,
            FocusSourceState::Unavailable(error) => Some(error),
        };
        let has_config = initial_config.is_some();
        let shared = Arc::new(Shared {
            config: RwLock::new(initial_config.map(Arc::new)),
            health: Mutex::new(RuntimeHealth {
                focus_error,
                refresh_error: None,
                dispatch_error: None,
                worker_error: None,
                has_config,
                lifecycle: RuntimeLifecycle::Starting,
            }),
            status,
            #[cfg(test)]
            status_history: Mutex::new(vec![RuntimeStatus::starting()]),
            hid_inventory,
            commands,
            shutdown,
            admission: Mutex::new(Admission {
                refresh_pending: true,
                shutdown_requested: false,
            }),
        });
        let runtime = Self { shared };
        let worker_runtime = runtime.clone();
        let worker = std::thread::Builder::new()
            .name("locked-in-automation".to_string())
            .spawn(move || {
                worker::run(
                    worker_runtime,
                    focus_events,
                    command_rx,
                    shutdown_rx,
                    Box::new(backend),
                );
                let _ = completed.send(());
            })?;
        let owner = RuntimeOwner {
            runtime: runtime.clone(),
            completion_rx,
            worker: Some(worker),
        };
        Ok((runtime, owner))
    }

    pub fn replace_config(&self, config: Config) {
        *self
            .shared
            .config
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Arc::new(config));
        self.update_health(|health| health.has_config = true);
    }

    #[cfg(test)]
    pub fn status(&self) -> RuntimeStatus {
        self.shared.status.borrow().clone()
    }

    pub fn subscribe_status(&self) -> watch::Receiver<RuntimeStatus> {
        self.shared.status.subscribe()
    }

    #[cfg(test)]
    pub fn hid_inventory(&self) -> Arc<HidInventory> {
        self.shared.hid_inventory.borrow().clone()
    }

    pub fn subscribe_hid_inventory(&self) -> watch::Receiver<Arc<HidInventory>> {
        self.shared.hid_inventory.subscribe()
    }

    pub async fn test_action(
        &self,
        action: SendAction,
        devices: Vec<Device>,
    ) -> std::result::Result<TestDispatchResult, RuntimeRequestError> {
        let receiver = self.admit_test_action(action, devices)?;
        receiver
            .await
            .unwrap_or(Err(RuntimeRequestError::Cancelled))
    }

    fn admit_test_action(
        &self,
        action: SendAction,
        devices: Vec<Device>,
    ) -> std::result::Result<
        oneshot::Receiver<std::result::Result<TestDispatchResult, RuntimeRequestError>>,
        RuntimeRequestError,
    > {
        let (response, receiver) = oneshot::channel();
        {
            let admission = self
                .shared
                .admission
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if admission.shutdown_requested {
                return Err(RuntimeRequestError::Unavailable);
            }
            match self.shared.commands.try_send(RuntimeCommand::TestAction {
                action,
                devices,
                response,
            }) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_)) => {
                    return Err(RuntimeRequestError::Busy);
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    return Err(RuntimeRequestError::Unavailable);
                }
            }
        }
        Ok(receiver)
    }

    pub fn request_hid_refresh(
        &self,
    ) -> std::result::Result<HidRefreshRequestResult, RuntimeRequestError> {
        let mut admission = self
            .shared
            .admission
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if admission.shutdown_requested {
            return Err(RuntimeRequestError::Unavailable);
        }
        if admission.refresh_pending {
            return Ok(HidRefreshRequestResult::AlreadyPending);
        }

        match self.shared.commands.try_send(RuntimeCommand::RefreshHid) {
            Ok(()) => admission.refresh_pending = true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                return Err(RuntimeRequestError::Busy);
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                return Err(RuntimeRequestError::Unavailable);
            }
        }
        Ok(HidRefreshRequestResult::Queued)
    }

    pub fn request_shutdown(&self) {
        let mut admission = self
            .shared
            .admission
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if admission.shutdown_requested {
            return;
        }
        admission.shutdown_requested = true;
        self.update_health(|health| {
            if health.lifecycle != RuntimeLifecycle::Stopped {
                health.lifecycle = RuntimeLifecycle::Stopping;
            }
        });
        self.shared.shutdown.send_replace(true);
    }

    fn config_snapshot(&self) -> Option<Arc<Config>> {
        self.shared
            .config
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn update_health(&self, update: impl FnOnce(&mut RuntimeHealth)) {
        let mut health = self
            .shared
            .health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        update(&mut health);
        let status = health.status();
        self.shared.status.send_replace(status.clone());
        #[cfg(test)]
        self.shared
            .status_history
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(status);
    }

    fn publish_hid_inventory(&self, inventory: HidInventory) {
        self.shared.hid_inventory.send_replace(Arc::new(inventory));
    }

    fn publish_hid_inventory_if_changed(&self, inventory: HidInventory) -> bool {
        let changed = self.shared.hid_inventory.borrow().as_ref() != &inventory;
        if changed {
            self.publish_hid_inventory(inventory);
        }
        changed
    }

    fn publish_completed_hid_refresh(&self, inventory: HidInventory) {
        let mut admission = self
            .shared
            .admission
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.publish_hid_inventory(inventory);
        admission.refresh_pending = false;
    }

    fn close_admission(&self) {
        let mut admission = self
            .shared
            .admission
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        admission.refresh_pending = false;
        admission.shutdown_requested = true;
    }

    #[cfg(test)]
    fn status_history(&self) -> Vec<RuntimeStatus> {
        self.shared
            .status_history
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

pub(crate) struct RuntimeOwner {
    runtime: AutomationRuntime,
    completion_rx: std::sync::mpsc::Receiver<()>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl RuntimeOwner {
    pub fn shutdown_and_join(mut self, timeout: Duration) {
        self.runtime.request_shutdown();
        if self.completion_rx.recv_timeout(timeout).is_ok() {
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        } else {
            crate::app_log::write_error(
                "automation runtime did not stop before the shutdown timeout",
            );
        }
    }
}

impl Drop for RuntimeOwner {
    fn drop(&mut self) {
        self.runtime.request_shutdown();
    }
}

#[cfg(test)]
mod tests;
