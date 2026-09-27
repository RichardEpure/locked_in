mod health;
mod inputs;
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
    config::{ActiveConfig, Device, SendAction},
    event::Event,
    hid::{HidBackend, HidInventory},
};

use health::RuntimeHealth;
pub(crate) use health::{RuntimePhase, RuntimeStatus};
pub(crate) use inputs::{EventSourceState, FocusInput, RuntimeInputs};
use worker::AutomationWorker;

#[cfg(test)]
use tests::ClaimGate;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TestDispatchResult {
    pub sent: usize,
    pub failures: Vec<String>,
}

struct Admission {
    refresh_pending: bool,
    shutdown_requested: bool,
}

enum BoundaryClaim {
    Shutdown,
    Event {
        event: Event,
        config: Option<Arc<ActiveConfig>>,
    },
    Command,
    Wait,
}

/// Cross-thread state only.
struct Shared {
    config: RwLock<Option<Arc<ActiveConfig>>>,
    health: RuntimeHealth,
    hid_inventory: watch::Sender<Arc<HidInventory>>,
    commands: mpsc::Sender<RuntimeCommand>,
    shutdown: watch::Sender<bool>,
    admission: Mutex<Admission>,
    #[cfg(test)]
    initialization_claim_gate: Mutex<Option<ClaimGate>>,
    #[cfg(test)]
    boundary_claim_gate: Mutex<Option<ClaimGate>>,
}

impl Shared {
    fn claim_initialization(&self, inputs: &RuntimeInputs) -> bool {
        let admission = self
            .admission
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !admission.shutdown_requested {
            return true;
        }
        // Shutdown won admission before initialization; cancel pending input under the same lock.
        inputs.cancel_pending();
        false
    }

    fn claim_boundary(&self, inputs: &mut RuntimeInputs, command_staged: bool) -> BoundaryClaim {
        // Hold admission and config through input claiming so shutdown, config replacement,
        // and starting an event have a consistent ordering. Input guards stay held through
        // the observation mark; later observations belong to a later batch. The claimed
        // batch retains its selected config after these locks are released.
        let admission = self
            .admission
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let config = self
            .config
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if admission.shutdown_requested {
            // Shutdown won admission; cancel pending input instead of starting it.
            inputs.cancel_pending();
            BoundaryClaim::Shutdown
        } else if let Some(event) = inputs.claim_next() {
            BoundaryClaim::Event {
                event,
                config: config.clone(),
            }
        } else if command_staged {
            BoundaryClaim::Command
        } else {
            BoundaryClaim::Wait
        }
    }

    fn close_admission(&self) {
        let mut admission = self
            .admission
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        admission.refresh_pending = false;
        admission.shutdown_requested = true;
    }
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

/// Cloneable caller handle. Requests execute on one dedicated automation worker.
#[derive(Clone)]
pub(crate) struct AutomationRuntime {
    shared: Arc<Shared>,
}

impl AutomationRuntime {
    pub fn start_active(
        initial_config: Option<Arc<ActiveConfig>>,
        inputs: RuntimeInputs,
        backend: impl HidBackend,
    ) -> Result<(Self, RuntimeOwner)> {
        Self::start_active_inner(
            initial_config,
            inputs,
            backend,
            #[cfg(test)]
            None,
        )
    }

    fn start_active_inner(
        initial_config: Option<Arc<ActiveConfig>>,
        inputs: RuntimeInputs,
        backend: impl HidBackend,
        #[cfg(test)] initialization_claim_gate: Option<ClaimGate>,
    ) -> Result<(Self, RuntimeOwner)> {
        let (commands, command_rx) = mpsc::channel(ORDINARY_COMMAND_CAPACITY);
        let (shutdown, shutdown_rx) = watch::channel(false);
        let (hid_inventory, _) = watch::channel(Arc::new(HidInventory::default()));
        let (completed, completion_rx) = std::sync::mpsc::channel();
        let health = RuntimeHealth::new(inputs.source_states(), initial_config.is_some());
        let shared = Arc::new(Shared {
            config: RwLock::new(initial_config),
            health,
            hid_inventory,
            commands,
            shutdown,
            admission: Mutex::new(Admission {
                refresh_pending: true,
                shutdown_requested: false,
            }),
            #[cfg(test)]
            initialization_claim_gate: Mutex::new(initialization_claim_gate),
            #[cfg(test)]
            boundary_claim_gate: Mutex::new(None),
        });
        let runtime = Self { shared };
        let worker = AutomationWorker::new(
            runtime.shared.clone(),
            inputs,
            command_rx,
            shutdown_rx,
            Box::new(backend),
        );
        let worker = std::thread::Builder::new()
            .name("locked-in-automation".to_string())
            .spawn(move || {
                worker.run();
                let _ = completed.send(());
            })?;
        let owner = RuntimeOwner {
            runtime: runtime.clone(),
            completion_rx,
            worker: Some(worker),
        };
        Ok((runtime, owner))
    }

    pub(crate) fn replace_active_config(&self, config: Arc<ActiveConfig>) {
        *self
            .shared
            .config
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(config);
        self.shared.health.config_installed();
    }

    pub fn subscribe_status(&self) -> watch::Receiver<RuntimeStatus> {
        self.shared.health.subscribe()
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
                Err(mpsc::error::TrySendError::Full(_)) => return Err(RuntimeRequestError::Busy),
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
            Err(mpsc::error::TrySendError::Full(_)) => return Err(RuntimeRequestError::Busy),
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
        self.shared.health.begin_stopping();
        self.shared.shutdown.send_replace(true);
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
