use std::sync::Arc;

use dioxus::{desktop::WindowCloseBehaviour, prelude::*};

use super::app::effective_close_behavior;
use crate::{app_log, config::PublishedConfig};

/// One UI projection. Readers cannot write its signal; successful mutations and
/// observer delivery both acknowledge through the same monotonic boundary.
#[derive(Clone, Copy)]
pub(crate) struct PublishedConfigContext {
    publication: Signal<Option<Arc<PublishedConfig>>>,
    close_behavior: Signal<WindowCloseBehaviour>,
    tray_available: bool,
}

impl PublishedConfigContext {
    pub(super) fn new(
        publication: Signal<Option<Arc<PublishedConfig>>>,
        close_behavior: Signal<WindowCloseBehaviour>,
        tray_available: bool,
    ) -> Self {
        let context = Self {
            publication,
            close_behavior,
            tray_available,
        };
        if let Some(initial) = context.publication.peek().as_ref() {
            context.apply_settings(initial);
        }
        context
    }

    pub(crate) fn current(self) -> Option<Arc<PublishedConfig>> {
        self.publication.read().clone()
    }

    pub(super) fn required(self) -> Arc<PublishedConfig> {
        self.current()
            .expect("configuration is available after successful bootstrap")
    }

    pub(super) fn acknowledge(mut self, publication: Arc<PublishedConfig>) {
        if self
            .publication
            .peek()
            .as_ref()
            .is_some_and(|current| current.revision() >= publication.revision())
        {
            return;
        }
        self.apply_settings(&publication);
        self.publication.set(Some(publication));
    }

    fn apply_settings(mut self, publication: &PublishedConfig) {
        let settings = &publication.editable().settings;
        app_log::set_level(settings.log_level);
        self.close_behavior.set(effective_close_behavior(
            settings.close_to_tray,
            self.tray_available,
        ));
    }
}

#[cfg(test)]
mod tests;
