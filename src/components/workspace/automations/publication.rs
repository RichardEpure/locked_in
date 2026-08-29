use std::{error::Error, fmt, sync::Arc};

use crate::{
    config::{Automation, ConfigCoordinator, ConfigCoordinatorError, PublishedConfig},
    focused_window::FocusedWindow,
};

use super::mutations::insert_captured_matcher;

#[derive(Debug)]
pub(in crate::components::workspace) enum AutomationCommitError {
    Coordinator(ConfigCoordinatorError),
    AutomationMissing(String),
    AutomationAlreadyExists(String),
    CaseMissing(String),
}

impl AutomationCommitError {
    pub(super) fn stale_actual_revision(&self) -> Option<u64> {
        match self {
            Self::Coordinator(ConfigCoordinatorError::StaleRevision { actual, .. }) => {
                Some(*actual)
            }
            _ => None,
        }
    }
}

impl fmt::Display for AutomationCommitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Coordinator(error) => error.fmt(formatter),
            Self::AutomationMissing(id) => {
                write!(formatter, "automation {id:?} no longer exists")
            }
            Self::AutomationAlreadyExists(id) => {
                write!(formatter, "automation {id:?} already exists")
            }
            Self::CaseMissing(id) => write!(formatter, "case {id:?} no longer exists"),
        }
    }
}

impl Error for AutomationCommitError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Coordinator(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ConfigCoordinatorError> for AutomationCommitError {
    fn from(error: ConfigCoordinatorError) -> Self {
        Self::Coordinator(error)
    }
}

pub(super) fn new_automation(publication: &PublishedConfig) -> Automation {
    Automation {
        id: publication.editable().next_id("automation"),
        ..Automation::default()
    }
}

pub(super) fn duplicate_automation(
    publication: &PublishedConfig,
    source: &Automation,
) -> Automation {
    let mut copy = source.clone();
    copy.id = publication.editable().next_id(&format!("{}-copy", copy.id));
    copy.name = format!("{} Copy", copy.name);
    copy.enabled = false;
    copy
}

pub(super) fn cancel_automation(
    publication: &PublishedConfig,
    automation_id: &str,
    is_new: bool,
) -> Option<Automation> {
    if is_new {
        return None;
    }
    publication
        .editable()
        .automations
        .iter()
        .find(|automation| automation.id == automation_id)
        .cloned()
}

pub(super) fn save_automation(
    coordinator: &ConfigCoordinator,
    expected_revision: u64,
    draft: &Automation,
    is_new: bool,
) -> Result<Arc<PublishedConfig>, AutomationCommitError> {
    let mut candidate = editable_at_revision(coordinator, expected_revision)?;
    if is_new {
        if candidate
            .automations
            .iter()
            .any(|automation| automation.id == draft.id)
        {
            return Err(AutomationCommitError::AutomationAlreadyExists(
                draft.id.clone(),
            ));
        }
        candidate.automations.push(draft.clone());
    } else {
        let Some(index) = candidate
            .automations
            .iter()
            .position(|automation| automation.id == draft.id)
        else {
            return Err(AutomationCommitError::AutomationMissing(draft.id.clone()));
        };
        candidate.automations[index] = draft.clone();
    }
    coordinator
        .update(expected_revision, move |_| candidate)
        .map_err(Into::into)
}

pub(super) fn delete_automation(
    coordinator: &ConfigCoordinator,
    expected_revision: u64,
    automation_id: &str,
) -> Result<Arc<PublishedConfig>, AutomationCommitError> {
    let mut candidate = editable_at_revision(coordinator, expected_revision)?;
    let original_len = candidate.automations.len();
    candidate
        .automations
        .retain(|automation| automation.id != automation_id);
    if candidate.automations.len() == original_len {
        return Err(AutomationCommitError::AutomationMissing(
            automation_id.to_string(),
        ));
    }
    coordinator
        .update(expected_revision, move |_| candidate)
        .map_err(Into::into)
}

pub(in crate::components::workspace) fn commit_captured_matcher(
    coordinator: &ConfigCoordinator,
    expected_revision: u64,
    automation_id: &str,
    case_id: &str,
    exception: bool,
    captured: &FocusedWindow,
) -> Result<Arc<PublishedConfig>, AutomationCommitError> {
    let mut candidate = editable_at_revision(coordinator, expected_revision)?;
    let Some(automation) = candidate
        .automations
        .iter_mut()
        .find(|automation| automation.id == automation_id)
    else {
        return Err(AutomationCommitError::AutomationMissing(
            automation_id.to_string(),
        ));
    };
    if insert_captured_matcher(automation, case_id, exception, captured).is_none() {
        return Err(AutomationCommitError::CaseMissing(case_id.to_string()));
    }
    coordinator
        .update(expected_revision, move |_| candidate)
        .map_err(Into::into)
}

fn editable_at_revision(
    coordinator: &ConfigCoordinator,
    expected_revision: u64,
) -> Result<crate::config::EditableConfig, AutomationCommitError> {
    let publication = coordinator.current();
    if publication.revision() != expected_revision {
        return Err(ConfigCoordinatorError::StaleRevision {
            expected: expected_revision,
            actual: publication.revision(),
        }
        .into());
    }
    Ok(publication.editable().as_ref().clone())
}

#[cfg(test)]
mod tests;
