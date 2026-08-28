use std::collections::HashSet;

use dioxus::prelude::*;

mod action_editor;
mod automation_editor;
mod automations_view;
mod case_editor;
mod condition_row;
mod matcher_editor;
mod matcher_group;
mod mutations;

pub(super) use automations_view::AutomationsView;
pub(in crate::components::workspace) use mutations::insert_captured_matcher;

static INVALID_REPORT_IDS: GlobalSignal<HashSet<String>> = Signal::global(HashSet::new);
