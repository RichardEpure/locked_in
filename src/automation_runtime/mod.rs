mod worker;

use std::{
    sync::{Arc, Mutex, RwLock, atomic::AtomicBool},
    time::Duration,
};

use anyhow::Result;
use tokio::sync::{mpsc, oneshot, watch};

use crate::{
    config::{Config, Device, SendAction},
    win::WindowMetadata,
};

pub(crate) trait ReportDispatcher: Send + 'static {
    fn initialize(&mut self) -> Result<()>;
    fn send_report(&mut self, device: &Device, report: &[u8]) -> Result<usize>;
}

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
    hid_error: Option<String>,
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
        if let Some(error) = &self.hid_error {
            return RuntimeStatus {
                phase: RuntimePhase::Degraded,
                detail: Some(error.clone()),
            };
        }
        RuntimeStatus {
            phase: RuntimePhase::Active,
            detail: None,
        }
    }
}

struct Shared {
    config: RwLock<Option<Arc<Config>>>,
    health: Mutex<RuntimeHealth>,
    status: watch::Sender<RuntimeStatus>,
    commands: mpsc::UnboundedSender<RuntimeCommand>,
    shutdown_requested: AtomicBool,
}

enum RuntimeCommand {
    TestAction {
        action: SendAction,
        devices: Vec<Device>,
        response: oneshot::Sender<TestDispatchResult>,
    },
    Shutdown,
}

#[derive(Clone)]
pub(crate) struct AutomationRuntime {
    shared: Arc<Shared>,
}

impl AutomationRuntime {
    pub fn start(
        initial_config: Option<Config>,
        focus_events: watch::Receiver<WindowMetadata>,
        focus_source: FocusSourceState,
        dispatcher: impl ReportDispatcher,
    ) -> Result<(Self, RuntimeOwner)> {
        let (commands, command_rx) = mpsc::unbounded_channel();
        let (status, _) = watch::channel(RuntimeStatus::starting());
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
                hid_error: None,
                worker_error: None,
                has_config,
                started: false,
                stopping: false,
                stopped: false,
            }),
            status,
            commands,
            shutdown_requested: AtomicBool::new(false),
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
                    Box::new(dispatcher),
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

    pub async fn test_action(
        &self,
        action: SendAction,
        devices: Vec<Device>,
    ) -> Result<TestDispatchResult> {
        if self
            .shared
            .shutdown_requested
            .load(std::sync::atomic::Ordering::Acquire)
        {
            anyhow::bail!("automation runtime is stopping");
        }
        let (response, receiver) = oneshot::channel();
        self.shared
            .commands
            .send(RuntimeCommand::TestAction {
                action,
                devices,
                response,
            })
            .map_err(|_| anyhow::anyhow!("automation runtime is unavailable"))?;
        receiver
            .await
            .map_err(|_| anyhow::anyhow!("automation runtime stopped before the test completed"))
    }

    pub fn request_shutdown(&self) {
        if self
            .shared
            .shutdown_requested
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            return;
        }
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
