use crate::{config::EventKind, focused_window::FocusedWindow};

/// An occurrence with its payload and source-specific tracking metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Event {
    FocusedWindowChanged {
        window: FocusedWindow,
        generation: u64,
    },
}

impl Event {
    pub fn kind(&self) -> EventKind {
        match self {
            Self::FocusedWindowChanged { .. } => EventKind::FocusedWindowChanged,
        }
    }
}
