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

struct RuntimeHealth {
    focus_error: Option<String>,
    refresh_error: Option<String>,
    dispatch_error: Option<String>,
    worker_error: Option<String>,
    has_config: bool,
    started: bool,
    stopping: bool,
    stopped: bool,
}

impl RuntimeHealth {
    fn status(&self) -> RuntimeStatus {
        if self.stopped {
            return RuntimeStatus {
                phase: RuntimePhase::Stopped,
                detail: None,
            };
        }
        if self.stopping {
            return RuntimeStatus {
                phase: RuntimePhase::Stopping,
                detail: None,
            };
        }
        if !self.started {
            return RuntimeStatus::starting();
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
    hid_inventory: watch::Sender<Arc<HidInventory>>,
    commands: mpsc::UnboundedSender<RuntimeCommand>,
    admission: Mutex<Admission>,
}

enum RuntimeCommand {
    TestAction {
        action: SendAction,
        devices: Vec<Device>,
        response: oneshot::Sender<TestDispatchResult>,
    },
    RefreshHid,
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HidRefreshRequestResult {
    Queued,
    AlreadyPending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RuntimeUnavailable;

impl Display for RuntimeUnavailable {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("automation runtime is stopping or unavailable")
    }
}

impl Error for RuntimeUnavailable {}

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
        let (commands, command_rx) = mpsc::unbounded_channel();
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
                started: false,
                stopping: false,
                stopped: false,
            }),
            status,
            hid_inventory,
            commands,
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
                worker::run(worker_runtime, focus_events, command_rx, Box::new(backend));
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
    ) -> Result<TestDispatchResult> {
        let (response, receiver) = oneshot::channel();
        {
            let admission = self
                .shared
                .admission
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if admission.shutdown_requested {
                return Err(RuntimeUnavailable.into());
            }
            self.shared
                .commands
                .send(RuntimeCommand::TestAction {
                    action,
                    devices,
                    response,
                })
                .map_err(|_| RuntimeUnavailable)?;
        }
        receiver
            .await
            .map_err(|_| anyhow::anyhow!("automation runtime stopped before the test completed"))
    }

    pub fn request_hid_refresh(
        &self,
    ) -> std::result::Result<HidRefreshRequestResult, RuntimeUnavailable> {
        let mut admission = self
            .shared
            .admission
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if admission.shutdown_requested {
            return Err(RuntimeUnavailable);
        }
        if admission.refresh_pending {
            return Ok(HidRefreshRequestResult::AlreadyPending);
        }

        admission.refresh_pending = true;
        if self
            .shared
            .commands
            .send(RuntimeCommand::RefreshHid)
            .is_err()
        {
            admission.refresh_pending = false;
            return Err(RuntimeUnavailable);
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
        self.update_health(|health| health.stopping = true);
        let _ = self.shared.commands.send(RuntimeCommand::Shutdown);
    }

    fn config_snapshot(&self) -> Option<Arc<Config>> {
        self.shared
            .config
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn update_health(&self, update: impl FnOnce(&mut RuntimeHealth)) {
        let status = {
            let mut health = self
                .shared
                .health
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            update(&mut health);
            health.status()
        };
        self.shared.status.send_replace(status);
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
