use std::collections::HashSet;

use dioxus::prelude::*;

mod action_editor;
mod automation_editor;
mod automations_view;
mod case_editor;
mod condition_row;
mod draft;
mod editing;
mod editor_helpers;
mod matcher_editor;
mod matcher_group;

pub(super) use automations_view::AutomationsView;
pub(in crate::components::workspace) use editing::commit_captured_matcher;

static INVALID_REPORT_IDS: GlobalSignal<HashSet<String>> = Signal::global(HashSet::new);
