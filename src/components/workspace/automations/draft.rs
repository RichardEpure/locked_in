use std::sync::Arc;

use crate::config::{Automation, ConfigCoordinator, PublishedConfig};

use super::publication::{
    AutomationCommitError, cancel_automation, delete_automation, save_automation,
};

/// The editable value and the two revisions that can diverge after a stale save.
/// Presentation state (messages, collapsed groups, raw report input) stays in the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AutomationDraft {
    pub(super) edited: Automation,
    baseline: Option<Automation>,
    baseline_revision: u64,
    next_attempt_revision: u64,
}

impl AutomationDraft {
    pub(super) fn create(revision: u64, automation: Automation) -> Self {
        Self {
            edited: automation,
            baseline: None,
            baseline_revision: revision,
            next_attempt_revision: revision,
        }
    }

    pub(super) fn edit(revision: u64, automation: Automation) -> Self {
        Self {
            baseline: Some(automation.clone()),
            ..Self::create(revision, automation)
        }
    }

    pub(super) fn is_new(&self) -> bool {
        self.baseline.is_none()
    }

    pub(super) fn is_dirty(&self) -> bool {
        self.baseline.as_ref() != Some(&self.edited)
    }

    pub(super) fn refresh_if_clean(&mut self, published: &PublishedConfig) -> bool {
        if self.is_dirty() || self.baseline_revision == published.revision() {
            return false;
        }
        self.cancel(published)
    }

    pub(super) fn cancel(&mut self, published: &PublishedConfig) -> bool {
        let Some(durable) = cancel_automation(published, &self.edited.id, self.is_new()) else {
            return false;
        };
        *self = Self::edit(published.revision(), durable);
        true
    }

    pub(super) fn save(
        &mut self,
        coordinator: &ConfigCoordinator,
    ) -> Result<Arc<PublishedConfig>, AutomationCommitError> {
        match save_automation(
            coordinator,
            self.next_attempt_revision,
            &self.edited,
            self.is_new(),
        ) {
            Ok(published) => {
                let saved = published
                    .editable()
                    .automations
                    .iter()
                    .find(|automation| automation.id == self.edited.id)
                    .expect("a successful save publishes the automation")
                    .clone();
                *self = Self::edit(published.revision(), saved);
                Ok(published)
            }
            Err(error) => {
                self.prepare_retry(&error);
                Err(error)
            }
        }
    }

    pub(super) fn delete(
        &mut self,
        coordinator: &ConfigCoordinator,
    ) -> Result<Option<Arc<PublishedConfig>>, AutomationCommitError> {
        if self.is_new() {
            return Ok(None);
        }
        match delete_automation(coordinator, self.next_attempt_revision, &self.edited.id) {
            Ok(published) => Ok(Some(published)),
            Err(error) => {
                self.prepare_retry(&error);
                Err(error)
            }
        }
    }

    fn prepare_retry(&mut self, error: &AutomationCommitError) {
        if let Some(actual) = error.stale_actual_revision() {
            // Keep the comparison baseline intact until save/cancel succeeds.
            self.next_attempt_revision = actual;
        }
    }
}

#[cfg(test)]
mod tests;
