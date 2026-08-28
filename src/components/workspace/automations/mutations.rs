use std::collections::HashSet;

use dioxus::prelude::*;

use crate::{
    config::{Automation, AutomationCase, SendAction, TextCondition, WindowMatcher},
    win,
};

pub(super) fn add_case(draft: &mut Signal<Automation>) {
    let snapshot = draft.read();
    let index = snapshot.cases.len() + 1;
    let id = next_child_id("case", snapshot.cases.iter().map(|case| case.id.as_str()));
    drop(snapshot);
    draft.write().cases.push(AutomationCase {
        id,
        name: format!("Case {index}"),
        ..AutomationCase::default()
    });
}

pub(super) fn add_matcher(draft: &mut Signal<Automation>, case_index: usize, exceptions: bool) {
    let case = &mut draft.write().cases[case_index];
    let id = next_child_id(
        "matcher",
        case.applications
            .iter()
            .chain(&case.exceptions)
            .map(|matcher| matcher.id.as_str()),
    );
    let list = if exceptions {
        &mut case.exceptions
    } else {
        &mut case.applications
    };
    list.push(WindowMatcher {
        id,
        ..WindowMatcher::default()
    });
}

pub(in crate::components::workspace) fn insert_captured_matcher(
    automation: &mut Automation,
    case_id: &str,
    exceptions: bool,
    captured: &win::WindowMetadata,
) -> Option<(String, usize)> {
    let case_index = automation
        .cases
        .iter()
        .position(|case| case.id == case_id)?;
    let case = &mut automation.cases[case_index];
    let matcher_id = next_child_id(
        "captured",
        case.applications
            .iter()
            .chain(&case.exceptions)
            .map(|matcher| matcher.id.as_str()),
    );
    let list = if exceptions {
        &mut case.exceptions
    } else {
        &mut case.applications
    };
    list.push(WindowMatcher {
        id: matcher_id,
        title: captured.title.clone().map(TextCondition::contains),
        class: captured.class.clone().map(TextCondition::contains),
        exe: captured
            .exe
            .as_ref()
            .map(|path| TextCondition::equals(path.to_string_lossy())),
    });
    Some((case.name.clone(), case_index))
}

pub(super) fn matcher_group_name(exceptions: bool) -> &'static str {
    if exceptions {
        "Except when"
    } else {
        "Applications"
    }
}

pub(super) fn matcher_group_body_id(case_index: usize, exceptions: bool) -> String {
    format!(
        "matcher-list-{case_index}-{}",
        if exceptions {
            "exceptions"
        } else {
            "applications"
        }
    )
}

pub(super) fn reveal_last_matcher(case_index: usize, exceptions: bool) {
    let body_id = matcher_group_body_id(case_index, exceptions);
    spawn(async move {
        let _ = document::eval(&format!(
            "requestAnimationFrame(() => requestAnimationFrame(() => document.getElementById('{body_id}')?.lastElementChild?.scrollIntoView({{ block: 'nearest' }})))"
        ))
        .await;
    });
}

pub(super) fn add_action(draft: &mut Signal<Automation>, case_index: Option<usize>) {
    let snapshot = draft.read();
    let id = next_child_id(
        "action",
        snapshot
            .cases
            .iter()
            .flat_map(|case| case.actions.iter())
            .chain(snapshot.otherwise_actions.iter())
            .map(|action| action.id.as_str()),
    );
    drop(snapshot);
    let action = SendAction {
        id,
        ..SendAction::default()
    };
    if let Some(index) = case_index {
        draft.write().cases[index].actions.push(action);
    } else {
        draft.write().otherwise_actions.push(action);
    }
}

pub(super) fn with_action_mut(
    draft: &mut Signal<Automation>,
    case_index: Option<usize>,
    action_index: usize,
    update: impl FnOnce(&mut SendAction),
) {
    let mut automation = draft.write();
    let action = if let Some(index) = case_index {
        &mut automation.cases[index].actions[action_index]
    } else {
        &mut automation.otherwise_actions[action_index]
    };
    update(action);
}

pub(super) fn remove_action(
    draft: &mut Signal<Automation>,
    case_index: Option<usize>,
    action_index: usize,
) {
    if let Some(index) = case_index {
        draft.write().cases[index].actions.remove(action_index);
    } else {
        draft.write().otherwise_actions.remove(action_index);
    }
}

fn next_child_id<'a>(prefix: &str, existing: impl Iterator<Item = &'a str>) -> String {
    let existing = existing.collect::<HashSet<_>>();
    (1..)
        .map(|index| format!("{prefix}-{index}"))
        .find(|candidate| !existing.contains(candidate.as_str()))
        .expect("identifier search is finite")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn automation_with_case() -> Automation {
        Automation {
            id: "automation-1".into(),
            cases: vec![AutomationCase {
                id: "case-1".into(),
                name: "Primary".into(),
                applications: vec![WindowMatcher {
                    id: "captured-1".into(),
                    ..WindowMatcher::default()
                }],
                ..AutomationCase::default()
            }],
            ..Automation::default()
        }
    }

    #[test]
    fn captured_matcher_routes_metadata_to_the_selected_group() {
        let mut automation = automation_with_case();
        let captured = win::WindowMetadata {
            title: Some("Editor".into()),
            class: Some("WindowClass".into()),
            exe: Some(std::path::PathBuf::from(r"C:\Apps\editor.exe")),
        };

        let case_name = insert_captured_matcher(&mut automation, "case-1", true, &captured);

        assert_eq!(case_name, Some(("Primary".into(), 0)));
        assert_eq!(automation.cases[0].applications.len(), 1);
        let matcher = &automation.cases[0].exceptions[0];
        assert_eq!(matcher.id, "captured-2");
        assert_eq!(matcher.title, Some(TextCondition::contains("Editor")));
        assert_eq!(matcher.class, Some(TextCondition::contains("WindowClass")));
        assert_eq!(
            matcher.exe,
            Some(TextCondition::equals(r"C:\Apps\editor.exe"))
        );
    }

    #[test]
    fn captured_matcher_leaves_automation_unchanged_when_case_is_missing() {
        let mut automation = automation_with_case();
        let original = automation.clone();

        let result = insert_captured_matcher(
            &mut automation,
            "missing-case",
            false,
            &win::WindowMetadata::default(),
        );

        assert_eq!(result, None);
        assert_eq!(automation, original);
    }
}
