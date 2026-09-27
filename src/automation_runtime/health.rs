use std::{collections::BTreeMap, sync::Mutex};

use tokio::sync::watch;

use super::EventSourceState;
use crate::config::EventKind;

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

/// Serializes state transitions and their publication, including transitions
/// originating on caller threads while the worker is blocked in HID I/O.
pub(super) struct RuntimeHealth {
    state: Mutex<HealthState>,
    status: watch::Sender<RuntimeStatus>,
}

impl RuntimeHealth {
    pub(super) fn new(sources: BTreeMap<EventKind, EventSourceState>, has_config: bool) -> Self {
        let (status, _) = watch::channel(RuntimeStatus::starting());
        Self {
            state: Mutex::new(HealthState {
                sources,
                refresh_error: None,
                dispatch_error: None,
                worker_error: None,
                has_config,
                lifecycle: RuntimeLifecycle::Starting,
                #[cfg(test)]
                history: vec![RuntimeStatus::starting()],
            }),
            status,
        }
    }

    pub(super) fn subscribe(&self) -> watch::Receiver<RuntimeStatus> {
        self.status.subscribe()
    }

    pub(super) fn startup_finished(&self) {
        self.update(|state| {
            if state.lifecycle == RuntimeLifecycle::Starting {
                state.lifecycle = RuntimeLifecycle::Running;
            }
        });
    }

    pub(super) fn source_changed(&self, source: EventKind, availability: EventSourceState) {
        self.update(|state| {
            state.sources.insert(source, availability);
        });
    }

    pub(super) fn refresh_completed(&self, error: Option<String>) {
        self.update(|state| state.refresh_error = error);
    }

    pub(super) fn dispatch_completed(&self, error: Option<String>) {
        self.update(|state| state.dispatch_error = error);
    }

    pub(super) fn worker_failed(&self, error: String) {
        self.update(|state| {
            state.worker_error = Some(error);
            state.lifecycle = match state.lifecycle {
                RuntimeLifecycle::Starting => RuntimeLifecycle::Running,
                RuntimeLifecycle::Stopping => RuntimeLifecycle::Stopped,
                lifecycle => lifecycle,
            };
        });
    }

    pub(super) fn begin_stopping(&self) {
        self.update(|state| {
            if state.lifecycle != RuntimeLifecycle::Stopped {
                state.lifecycle = RuntimeLifecycle::Stopping;
            }
        });
    }

    pub(super) fn mark_stopped(&self) {
        self.update(|state| state.lifecycle = RuntimeLifecycle::Stopped);
    }

    fn update(&self, update: impl FnOnce(&mut HealthState)) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        update(&mut state);
        let status = state.status();
        self.status.send_replace(status.clone());
        #[cfg(test)]
        state.history.push(status);
    }

    #[cfg(test)]
    pub(super) fn history(&self) -> Vec<RuntimeStatus> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .history
            .clone()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeLifecycle {
    Starting,
    Running,
    Stopping,
    Stopped,
}

struct HealthState {
    sources: BTreeMap<EventKind, EventSourceState>,
    refresh_error: Option<String>,
    dispatch_error: Option<String>,
    worker_error: Option<String>,
    has_config: bool,
    lifecycle: RuntimeLifecycle,
    #[cfg(test)]
    history: Vec<RuntimeStatus>,
}

impl HealthState {
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
        let source_errors = self
            .sources
            .iter()
            .filter_map(|(source, state)| match state {
                EventSourceState::Available => None,
                EventSourceState::Unavailable(error) => Some(format!("{source}: {error}")),
            })
            .collect::<Vec<_>>();
        if !self
            .sources
            .values()
            .any(|state| *state == EventSourceState::Available)
        {
            if source_errors.is_empty() {
                unavailable.push("event sources are unavailable".to_string());
            } else {
                unavailable.extend(source_errors.iter().cloned());
            }
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
        let degraded = source_errors
            .into_iter()
            .chain(
                self.refresh_error
                    .iter()
                    .chain(&self.dispatch_error)
                    .cloned(),
            )
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
