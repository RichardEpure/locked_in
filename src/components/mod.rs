use std::sync::Arc;

use dioxus::prelude::{ReadSignal, Signal};

use crate::config::PublishedConfig;

mod app;
mod armed_capture_shortcut;
mod capture_shortcut;
mod workspace;

pub(crate) use app::App;

#[derive(Clone, Copy)]
pub(crate) struct PublishedConfigContext(ReadSignal<Option<Arc<PublishedConfig>>>);

impl PublishedConfigContext {
    fn new(publication: Signal<Option<Arc<PublishedConfig>>>) -> Self {
        Self(ReadSignal::new(publication))
    }

    pub(crate) fn current(self) -> Option<Arc<PublishedConfig>> {
        self.0()
    }
}
