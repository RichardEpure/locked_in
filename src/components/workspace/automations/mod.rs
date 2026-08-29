use std::{collections::HashSet, sync::Arc};

use dioxus::prelude::*;

use crate::config::{ConfigCoordinator, PublishedConfig};

mod action_editor;
mod automation_editor;
mod automations_view;
mod case_editor;
mod condition_row;
mod matcher_editor;
mod matcher_group;
mod mutations;
mod publication;

pub(super) use automations_view::AutomationsView;
pub(in crate::components::workspace) use publication::commit_captured_matcher;

static INVALID_REPORT_IDS: GlobalSignal<HashSet<String>> = Signal::global(HashSet::new);

pub(in crate::components::workspace) fn use_config_publication()
-> (Arc<ConfigCoordinator>, Signal<Arc<PublishedConfig>>) {
    let coordinator = consume_context::<Option<Arc<ConfigCoordinator>>>()
        .expect("configuration coordinator is available after bootstrap");
    let mut receiver =
        consume_context::<Option<tokio::sync::watch::Receiver<Arc<PublishedConfig>>>>()
            .expect("configuration publication subscription is available after bootstrap");
    let initial = receiver.borrow_and_update().clone();
    let mut publication = use_signal(move || initial);
    use_future(move || {
        let mut receiver = receiver.clone();
        async move {
            while receiver.changed().await.is_ok() {
                publication.set(receiver.borrow_and_update().clone());
            }
        }
    });
    (coordinator, publication)
}
