use std::sync::Arc;

use dioxus::prelude::*;

use crate::config::PublishedConfig;

#[derive(Clone, Copy)]
pub(super) struct PublishedConfigContext(ReadSignal<Arc<PublishedConfig>>);

impl PublishedConfigContext {
    pub(super) fn new(publication: Signal<Arc<PublishedConfig>>) -> Self {
        Self(ReadSignal::new(publication))
    }

    pub(super) fn current(self) -> Arc<PublishedConfig> {
        self.0()
    }
}
