use super::*;
use crate::config::{Automation, AutomationCase, TextCondition};

fn device() -> Device {
    Device {
        id: "keyboard".into(),
        name: "Keyboard".into(),
        report_length: 32,
        ..Device::default()
    }
}

fn config() -> EditableConfig {
    EditableConfig {
        devices: vec![device()],
        automations: vec![Automation {
            id: "layers".into(),
            name: "Layers".into(),
            enabled: true,
            cases: vec![AutomationCase {
                id: "gaming".into(),
                applications: vec![WindowMatcher {
                    id: "game".into(),
                    title: Some(TextCondition::contains("Game")),
                    ..WindowMatcher::default()
                }],
                actions: vec![SendAction {
                    id: "action".into(),
                    report: vec![0x87],
                    device_ids: vec!["keyboard".into()],
                    ..SendAction::default()
                }],
                ..AutomationCase::default()
            }],
            ..Automation::default()
        }],
        ..EditableConfig::default()
    }
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
    let mut config = EditableConfig {
        automations: vec![Automation {
            id: "draft".into(),
            ..Automation::default()
        }],
        ..EditableConfig::default()
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
fn duplicate_child_ids_are_rejected() {
    let mut config = config();
    let duplicate = config.automations[0].cases[0].applications[0].clone();
    config.automations[0].cases[0].applications.push(duplicate);

    let errors = config.validate();

    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("matcher ids must be unique"))
    );
}
