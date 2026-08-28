use std::path::PathBuf;

use super::*;

fn device() -> Device {
    Device {
        id: "keyboard".into(),
        name: "Keyboard".into(),
        report_length: 32,
        ..Device::default()
    }
}

fn action(report: &[u8]) -> SendAction {
    SendAction {
        id: "action".into(),
        report: report.to_vec(),
        device_ids: vec!["keyboard".into()],
        ..SendAction::default()
    }
}

fn matcher(title: &str) -> WindowMatcher {
    WindowMatcher {
        id: title.into(),
        title: Some(TextCondition::contains(title)),
        ..WindowMatcher::default()
    }
}

fn config() -> Config {
    Config {
        devices: vec![device()],
        automations: vec![Automation {
            id: "layers".into(),
            name: "Layers".into(),
            enabled: true,
            cases: vec![AutomationCase {
                id: "gaming".into(),
                name: "Gaming".into(),
                applications: vec![matcher("Game")],
                actions: vec![action(&[0x87])],
                ..AutomationCase::default()
            }],
            otherwise_actions: vec![action(&[0x86])],
            ..Automation::default()
        }],
        ..Config::default()
    }
}

#[test]
fn first_matching_case_wins_and_otherwise_is_fallback() {
    let config = config();
    let matching = WindowMetadata {
        title: Some("My Game".into()),
        ..WindowMetadata::default()
    };
    let other = WindowMetadata {
        title: Some("Editor".into()),
        ..WindowMetadata::default()
    };

    assert_eq!(config.evaluate_window(&matching)[0].action.report, [0x87]);
    assert_eq!(config.evaluate_window(&other)[0].action.report, [0x86]);
}

#[test]
fn matcher_fields_are_anded_and_entries_are_ored() {
    let mut config = config();
    config.automations[0].cases[0].applications = vec![
        WindowMatcher {
            id: "specific".into(),
            title: Some(TextCondition::contains("League")),
            exe: Some(TextCondition::equals(r"C:\Games\League.exe")),
            ..WindowMatcher::default()
        },
        matcher("Fallback"),
    ];
    let browser = WindowMetadata {
        title: Some("Watching League".into()),
        exe: Some(PathBuf::from(r"C:\Browser.exe")),
        ..WindowMetadata::default()
    };
    let game = WindowMetadata {
        title: Some("League".into()),
        exe: Some(PathBuf::from(r"C:\Games\League.exe")),
        ..WindowMetadata::default()
    };

    assert_eq!(config.evaluate_window(&browser)[0].action.report, [0x86]);
    assert_eq!(config.evaluate_window(&game)[0].action.report, [0x87]);
}

#[test]
fn matching_exception_skips_to_next_case() {
    let mut config = config();
    config.automations[0].cases[0]
        .exceptions
        .push(matcher("Browser"));
    let window = WindowMetadata {
        title: Some("Browser Game".into()),
        ..WindowMetadata::default()
    };

    assert_eq!(config.evaluate_window(&window)[0].action.report, [0x86]);
}

#[test]
fn all_automations_evaluate_independently() {
    let mut config = config();
    let mut second = config.automations[0].clone();
    second.id = "lighting".into();
    second.name = "Lighting".into();
    config.automations.push(second);
    let window = WindowMetadata {
        title: Some("Game".into()),
        ..WindowMetadata::default()
    };

    assert_eq!(config.evaluate_window(&window).len(), 2);
}

#[test]
fn validation_reports_regex_references_and_report_sizes() {
    let mut config = config();
    config.automations[0].cases[0].applications[0].title = Some(TextCondition {
        operator: MatchOperator::Regex,
        value: "[".into(),
        case_sensitive: false,
    });
    config.automations[0].cases[0].actions[0]
        .device_ids
        .push("missing".into());
    config.automations[0].cases[0].actions[0].report = vec![0; 33];

    let errors = config.validate();
    assert!(errors.iter().any(|error| error.path.ends_with("title")));
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("unknown device"))
    );
    assert!(errors.iter().any(|error| error.message.contains("exceeds")));
}

#[test]
fn disabled_automation_can_be_incomplete() {
    let mut config = Config {
        automations: vec![Automation {
            id: "draft".into(),
            ..Automation::default()
        }],
        ..Config::default()
    };
    config.automations[0].cases.push(AutomationCase {
        id: "unfinished-case".into(),
        applications: vec![WindowMatcher {
            id: "unfinished-matcher".into(),
            ..WindowMatcher::default()
        }],
        actions: vec![SendAction {
            id: "unfinished-action".into(),
            ..SendAction::default()
        }],
        ..AutomationCase::default()
    });
    assert!(config.validate().is_empty());
}

#[test]
fn legacy_config_is_rejected_instead_of_treated_as_empty() {
    let legacy = r#"
            [[rules]]
            name = "Legacy rule"
        "#;
    assert!(toml::from_str::<Config>(legacy).is_err());
}

#[test]
fn unknown_nested_fields_are_rejected() {
    let config = r#"
            version = 2

            [settings]
            close_to_try = true
        "#;
    assert!(toml::from_str::<Config>(config).is_err());
}

#[test]
fn duplicate_child_ids_are_rejected() {
    let mut config = config();
    config.automations[0].cases[0]
        .applications
        .push(matcher("Game"));
    let errors = config.validate();
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("matcher ids must be unique"))
    );
}
