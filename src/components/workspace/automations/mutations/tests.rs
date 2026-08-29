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
    let captured = FocusedWindow {
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
        &FocusedWindow::default(),
    );

    assert_eq!(result, None);
    assert_eq!(automation, original);
}
