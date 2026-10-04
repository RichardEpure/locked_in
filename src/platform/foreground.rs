use std::sync::{LazyLock, Mutex};

use anyhow::Result;
use tokio::sync::watch;

use crate::focused_window::{FocusedWindow, ForegroundObservation};

use super::native;

static FOREGROUND: LazyLock<Foreground> = LazyLock::new(Foreground::new);

/// Current state and subscriptions are projections of the same latest-state publication.
#[derive(Clone)]
pub(crate) struct Foreground {
    observations: watch::Sender<ForegroundObservation>,
}

impl Foreground {
    fn new() -> Self {
        let (observations, _) = watch::channel(ForegroundObservation::default());
        Self { observations }
    }

    pub(crate) fn current(&self) -> ForegroundObservation {
        self.observations.borrow().clone()
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<ForegroundObservation> {
        self.observations.subscribe()
    }
}

pub(crate) fn get() -> &'static Foreground {
    &FOREGROUND
}

/// Retain on the registering desktop thread until the event loop is destroyed.
/// Dropping this owner stops native observation; the last publication remains readable.
pub(crate) struct Monitor {
    _native: native::foreground::Monitor,
}

pub(crate) fn start() -> Result<Monitor> {
    native::foreground::start().map(|monitor| Monitor { _native: monitor })
}

pub(super) fn start_monitor<H>(
    register: impl FnOnce() -> Result<H>,
    reconcile: impl FnOnce(),
) -> Result<H> {
    // Acquire the owner before sampling: callbacks can arrive during registration,
    // and a failed registration must not publish a misleading initial observation.
    let owner = register()?;
    reconcile();
    Ok(owner)
}

pub(super) struct Ticket {
    sequence: u64,
}

struct ObservationState<I> {
    latest_sequence: u64,
    generation: u64,
    last_identity: Option<I>,
}

impl<I: Eq> ObservationState<I> {
    fn begin(&mut self, identity: I) -> Option<Ticket> {
        if self.last_identity.replace(identity).as_ref() == self.last_identity.as_ref() {
            return None;
        }
        self.latest_sequence = self
            .latest_sequence
            .checked_add(1)
            .expect("foreground observation sequence overflow");
        Some(Ticket {
            sequence: self.latest_sequence,
        })
    }

    fn complete(
        &mut self,
        ticket: Ticket,
        window: Option<FocusedWindow>,
    ) -> Option<ForegroundObservation> {
        if ticket.sequence != self.latest_sequence {
            return None;
        }
        let window = window?;
        self.generation = self
            .generation
            .checked_add(1)
            .expect("foreground observation generation overflow");
        Some(ForegroundObservation {
            generation: self.generation,
            window,
        })
    }
}

/// Adapter-only observation input. Native identity never enters the published type.
/// Metadata may complete later or be filtered, but only the latest accepted input can publish.
pub(super) struct Publisher<I> {
    state: Mutex<ObservationState<I>>,
    foreground: Foreground,
}

impl<I: Eq> Publisher<I> {
    pub(super) fn new(foreground: Foreground) -> Self {
        Self {
            state: Mutex::new(ObservationState {
                latest_sequence: 0,
                generation: 0,
                last_identity: None,
            }),
            foreground,
        }
    }

    pub(super) fn begin(&self, identity: I) -> Option<Ticket> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .begin(identity)
    }

    pub(super) fn begin_with<T>(
        &self,
        sample: impl FnOnce() -> Option<(I, T)>,
    ) -> Option<(Ticket, T)> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (identity, value) = sample()?;
        Some((state.begin(identity)?, value))
    }

    pub(super) fn complete(&self, ticket: Ticket, window: Option<FocusedWindow>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(observation) = state.complete(ticket, window) {
            // Keep completion and watch publication under the same lock so an older
            // completion cannot publish after a newer generation.
            self.foreground.observations.send_replace(observation);
        }
    }
}

#[cfg(test)]
mod tests;
