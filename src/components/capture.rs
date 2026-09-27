use dioxus::prelude::*;

use crate::focused_window::FocusedWindow;

static SESSION: GlobalSignal<CaptureSession> = Signal::global(CaptureSession::default);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CaptureTarget {
    pub(super) automation_id: String,
    pub(super) case_id: String,
    pub(super) exception: bool,
}

impl CaptureTarget {
    pub(super) fn new(automation_id: String, case_id: String, exception: bool) -> Self {
        Self {
            automation_id,
            case_id,
            exception,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CapturedWindow {
    pub(super) target: Option<CaptureTarget>,
    pub(super) window: FocusedWindow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Phase {
    Idle { target_removed: bool },
    Armed { target: Option<CaptureTarget> },
    Captured(CapturedWindow),
}

impl Default for Phase {
    fn default() -> Self {
        Self::Idle {
            target_removed: false,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct CaptureSession {
    generation: u64,
    phase: Phase,
}

impl CaptureSession {
    pub(super) fn generation(&self) -> u64 {
        self.generation
    }

    pub(super) fn is_armed(&self) -> bool {
        matches!(self.phase, Phase::Armed { .. })
    }

    pub(super) fn target(&self) -> Option<&CaptureTarget> {
        match &self.phase {
            Phase::Idle { .. } => None,
            Phase::Armed { target } => target.as_ref(),
            Phase::Captured(captured) => captured.target.as_ref(),
        }
    }

    pub(super) fn captured(&self) -> Option<&CapturedWindow> {
        match &self.phase {
            Phase::Captured(captured) => Some(captured),
            _ => None,
        }
    }

    pub(super) fn cancellation_message(&self) -> Option<&'static str> {
        matches!(
            self.phase,
            Phase::Idle {
                target_removed: true
            }
        )
        .then_some("Capture cancelled because the target automation or case no longer exists")
    }

    fn arm(&mut self, target: Option<CaptureTarget>) -> u64 {
        self.advance();
        self.phase = Phase::Armed { target };
        self.generation
    }

    fn capture(&mut self, generation: u64, window: FocusedWindow) -> bool {
        if generation != self.generation {
            return false;
        }
        let Phase::Armed { target } = &self.phase else {
            return false;
        };
        self.phase = Phase::Captured(CapturedWindow {
            target: target.clone(),
            window,
        });
        true
    }

    fn cancel(&mut self, generation: u64) {
        if generation == self.generation && !matches!(self.phase, Phase::Idle { .. }) {
            self.advance();
            self.phase = Phase::default();
        }
    }

    fn target_removed(&mut self, generation: u64) {
        if generation == self.generation && self.target().is_some() {
            self.cancel(generation);
            self.phase = Phase::Idle {
                target_removed: true,
            };
        }
    }

    fn expire(&mut self, generation: u64) {
        if self.is_armed() {
            self.cancel(generation);
        }
    }

    fn take_targeted(&mut self, generation: u64, target: &CaptureTarget) -> Option<FocusedWindow> {
        if generation != self.generation {
            return None;
        }
        let captured = self.captured()?;
        if captured.target.as_ref() != Some(target) {
            return None;
        }
        let window = captured.window.clone();
        self.cancel(generation);
        Some(window)
    }

    fn advance(&mut self) {
        self.generation = self
            .generation
            .checked_add(1)
            .expect("capture generation overflow");
    }
}

pub(super) fn session() -> CaptureSession {
    SESSION.read().clone()
}

pub(super) fn arm(target: Option<CaptureTarget>) {
    let generation = SESSION.write().arm(target);
    spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
        SESSION.write().expire(generation);
    });
}

pub(super) fn capture(generation: u64, window: FocusedWindow) -> bool {
    SESSION.write().capture(generation, window)
}

pub(super) fn cancel(generation: u64) {
    SESSION.write().cancel(generation);
}

pub(super) fn target_removed(generation: u64) {
    SESSION.write().target_removed(generation);
}

/// Consuming before insertion prevents reentrant effects from inserting twice.
pub(super) fn take_targeted(generation: u64, target: &CaptureTarget) -> Option<FocusedWindow> {
    SESSION.write().take_targeted(generation, target)
}

#[cfg(test)]
mod tests;
