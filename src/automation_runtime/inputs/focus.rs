use std::sync::{Arc, Mutex};

use tokio::sync::watch;

use super::EventSourceState;
use crate::{event::Event, focused_window::ForegroundObservation};

/// Focus represents latest desired state, so pending observations may coalesce.
pub(crate) struct FocusInput {
    observations: watch::Receiver<ForegroundObservation>,
    state: EventSourceState,
    open: bool,
    markers: Arc<Mutex<FocusGenerationMarkers>>,
}

impl FocusInput {
    pub fn new(
        observations: watch::Receiver<ForegroundObservation>,
        state: EventSourceState,
    ) -> Self {
        Self {
            observations,
            state,
            open: true,
            markers: Arc::new(Mutex::new(FocusGenerationMarkers::default())),
        }
    }

    #[cfg(test)]
    pub(in crate::automation_runtime) fn progress(&self) -> FocusProgress {
        FocusProgress {
            observations: self.observations.clone(),
            markers: Arc::clone(&self.markers),
        }
    }

    pub(super) fn state(&self) -> &EventSourceState {
        &self.state
    }

    /// Claims the latest unhandled observation and records its generation as started.
    pub(super) fn claim_next(&mut self) -> Option<Event> {
        let observation = self.observations.borrow_and_update();
        let mut markers = self
            .markers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if observation.generation <= markers.latest_handled.unwrap_or(0) {
            return None;
        }
        markers.latest_started = Some(observation.generation);
        Some(Event::FocusedWindowChanged {
            window: observation.window.clone(),
            generation: observation.generation,
        })
    }

    /// Records the latest unhandled generation as cancelled.
    pub(super) fn cancel_pending(&self) {
        let observation = self.observations.borrow();
        let mut markers = self
            .markers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if observation.generation > markers.latest_handled.unwrap_or(0) {
            markers.latest_cancelled = Some(observation.generation);
        }
    }

    pub(super) fn mark_handled(&self, generation: u64) {
        self.markers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .latest_handled = Some(generation);
    }

    pub(super) async fn changed(&mut self) -> Option<EventSourceState> {
        if !self.open {
            return std::future::pending().await;
        }
        if self.observations.changed().await.is_err() {
            self.open = false;
            self.state = EventSourceState::Unavailable("event source closed".to_string());
            Some(self.state.clone())
        } else {
            None
        }
    }

    #[cfg(test)]
    pub(super) fn has_pending(&self) -> bool {
        let observation = self.observations.borrow();
        let markers = self
            .markers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        observation.generation > markers.latest_handled.unwrap_or(0)
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::automation_runtime) struct FocusGenerationProgress {
    pub latest_observed: u64,
    pub latest_started: Option<u64>,
    pub latest_handled: Option<u64>,
    pub latest_cancelled: Option<u64>,
}

#[derive(Default)]
struct FocusGenerationMarkers {
    latest_started: Option<u64>,
    latest_handled: Option<u64>,
    latest_cancelled: Option<u64>,
}

/// Read-only access to live focus diagnostics, independent of the worker's input ownership.
#[cfg(test)]
pub(in crate::automation_runtime) struct FocusProgress {
    observations: watch::Receiver<ForegroundObservation>,
    markers: Arc<Mutex<FocusGenerationMarkers>>,
}

#[cfg(test)]
impl FocusProgress {
    pub(in crate::automation_runtime) fn snapshot(&self) -> FocusGenerationProgress {
        let observation = self.observations.borrow();
        let markers = self
            .markers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        FocusGenerationProgress {
            latest_observed: observation.generation,
            latest_started: markers.latest_started,
            latest_handled: markers.latest_handled,
            latest_cancelled: markers.latest_cancelled,
        }
    }
}
