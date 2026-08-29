use std::{error::Error, fmt, sync::Arc};

use crate::config::{
    ConfigCoordinator, ConfigCoordinatorError, Device, EditableConfig, PublishedConfig,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DeviceDraft {
    revision: u64,
    durable: Option<Device>,
    pub(super) edited: Device,
}

impl DeviceDraft {
    pub(super) fn create(revision: u64, device: Device) -> Self {
        Self {
            revision,
            durable: None,
            edited: device,
        }
    }

    pub(super) fn edit(revision: u64, device: Device) -> Self {
        Self {
            revision,
            durable: Some(device.clone()),
            edited: device,
        }
    }

    pub(super) fn is_new(&self) -> bool {
        self.durable.is_none()
    }

    pub(super) fn is_dirty(&self) -> bool {
        self.durable.as_ref() != Some(&self.edited)
    }

    pub(super) fn refresh_if_clean(&mut self, published: &PublishedConfig) -> bool {
        if self.is_new() || self.is_dirty() || self.revision == published.revision() {
            return false;
        }

        let previous = self.clone();
        self.cancel(published) && *self != previous
    }

    pub(super) fn is_acknowledged_by(&self, published: &PublishedConfig) -> bool {
        !self.is_new() && published.revision() >= self.revision
    }

    pub(super) fn save(
        &mut self,
        coordinator: &ConfigCoordinator,
    ) -> Result<Arc<PublishedConfig>, ConfigCoordinatorError> {
        let id = self.edited.id.clone();
        let edited = self.edited.clone();
        let is_new = self.is_new();
        let published = coordinator.update(self.revision, move |current| {
            let mut candidate = current.clone();
            if is_new {
                candidate.devices.push(edited);
            } else {
                let index = candidate
                    .devices
                    .iter()
                    .position(|device| device.id == id)
                    .expect("a device cannot disappear from an unchanged revision");
                candidate.devices[index] = edited;
            }
            candidate
        })?;
        self.rebase(&published);
        Ok(published)
    }

    pub(super) fn cancel(&mut self, published: &PublishedConfig) -> bool {
        let Some(device) = published
            .editable()
            .devices
            .iter()
            .find(|device| device.id == self.edited.id)
            .cloned()
        else {
            return false;
        };
        self.revision = published.revision();
        self.durable = Some(device.clone());
        self.edited = device;
        true
    }

    pub(super) fn delete(
        &self,
        coordinator: &ConfigCoordinator,
        references: &[String],
    ) -> Result<Option<Arc<PublishedConfig>>, DeviceDeleteError> {
        if self.is_new() {
            return Ok(None);
        }
        if !references.is_empty() {
            return Err(DeviceDeleteError::Referenced(references.to_vec()));
        }

        let id = self.edited.id.clone();
        coordinator
            .update(self.revision, move |current| {
                let mut candidate = current.clone();
                candidate.devices.retain(|device| device.id != id);
                candidate
            })
            .map(Some)
            .map_err(DeviceDeleteError::Coordinator)
    }

    fn rebase(&mut self, published: &PublishedConfig) {
        let device = published
            .editable()
            .devices
            .iter()
            .find(|device| device.id == self.edited.id)
            .cloned()
            .expect("a successful device save must publish the saved device");
        self.revision = published.revision();
        self.durable = Some(device.clone());
        self.edited = device;
    }
}

pub(super) fn clear_published_pending(
    pending: &mut Option<DeviceDraft>,
    published: &PublishedConfig,
) -> bool {
    if pending
        .as_ref()
        .is_some_and(|draft| draft.is_acknowledged_by(published))
    {
        *pending = None;
        return true;
    }
    false
}

#[derive(Debug)]
pub(super) enum DeviceDeleteError {
    Referenced(Vec<String>),
    Coordinator(ConfigCoordinatorError),
}

impl fmt::Display for DeviceDeleteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Referenced(references) => write!(formatter, "Used by: {}", references.join(", ")),
            Self::Coordinator(error) => write!(formatter, "Delete failed: {error}"),
        }
    }
}

impl Error for DeviceDeleteError {}

pub(super) fn device_references(config: &EditableConfig, device_id: &str) -> Vec<String> {
    config
        .automations
        .iter()
        .filter(|automation| {
            automation
                .cases
                .iter()
                .flat_map(|case| &case.actions)
                .chain(&automation.otherwise_actions)
                .any(|action| action.device_ids.iter().any(|id| id == device_id))
        })
        .map(|automation| automation.name.clone())
        .collect()
}

#[cfg(test)]
mod tests;
