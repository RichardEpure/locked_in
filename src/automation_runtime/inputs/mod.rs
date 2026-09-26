mod focus;

use std::collections::BTreeMap;

use crate::{config::EventKind, event::Event};

pub(crate) use focus::FocusInput;
#[cfg(test)]
pub(super) use focus::{FocusGenerationProgress, FocusProgress};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EventSourceState {
    Available,
    Unavailable(String),
}

/// Coordinates sources while each input owns its delivery policy and progress.
pub(crate) struct RuntimeInputs {
    pub focused_window: FocusInput,
}

impl RuntimeInputs {
    pub(super) fn source_states(&self) -> BTreeMap<EventKind, EventSourceState> {
        [(
            EventKind::FocusedWindowChanged,
            self.focused_window.state().clone(),
        )]
        .into()
    }

    /// Called only while admission and the configuration read guard are held.
    pub(super) fn claim_next(&mut self) -> Option<Event> {
        self.focused_window.claim_next()
    }

    /// Called under admission after shutdown has won the claim.
    pub(super) fn cancel_pending(&self) {
        self.focused_window.cancel_pending();
    }

    pub(super) fn mark_handled(&self, event: Event) {
        match event {
            Event::FocusedWindowChanged { generation, .. } => {
                self.focused_window.mark_handled(generation);
            }
        }
    }

    /// Wakes the worker without claiming an event ahead of its atomic batch boundary.
    pub(super) async fn changed(&mut self) -> Option<(EventKind, EventSourceState)> {
        self.focused_window
            .changed()
            .await
            .map(|state| (EventKind::FocusedWindowChanged, state))
    }

    #[cfg(test)]
    pub(super) fn has_pending(&self) -> bool {
        self.focused_window.has_pending()
    }
}
