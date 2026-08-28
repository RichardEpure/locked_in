use std::path::PathBuf;

use super::*;
use crate::{
    config::{Automation, AutomationCase},
    focused_window::FocusedWindow,
};

fn device() -> Device {
    Device {
        id: "keyboard".into(),
        name: "Keyboard".into(),
        report_length: 32,
        ..Device::default()
    }
}

fn action(id: &str, report: u8) -> SendAction {
    SendAction {
        id: id.into(),
        report: vec![report],
        device_ids: vec!["keyboard".into()],
        ..SendAction::default()
    }
}

fn matcher(id: &str, title: &str) -> WindowMatcher {
    WindowMatcher {
        id: id.into(),
        title: Some(TextCondition::contains(title)),
        ..WindowMatcher::default()
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
                name: "Gaming".into(),
                applications: vec![matcher("game", "Game")],
                actions: vec![action("game-action", 0x87)],
                ..AutomationCase::default()
            }],
            otherwise_actions: vec![action("otherwise-action", 0x86)],
            ..Automation::default()
        }],
        ..EditableConfig::default()
    }
}

#[test]
fn first_matching_case_wins_and_otherwise_is_fallback() {
    let mut config = config();
    config.automations[0].cases.push(AutomationCase {
        id: "second".into(),
        name: "Second".into(),
        applications: vec![matcher("second", "Game")],
        actions: vec![action("second-action", 0x88)],
        ..AutomationCase::default()
    });

    let matching = FocusedWindow {
        title: Some("My Game".into()),
        ..FocusedWindow::default()
    };
    let other = FocusedWindow {
        title: Some("Editor".into()),
        ..FocusedWindow::default()
    };

    assert_eq!(config.evaluate_window(&matching)[0].action.report, [0x87]);
    assert_eq!(config.evaluate_window(&other)[0].action.report, [0x86]);
}

#[test]
fn neutral_focus_fields_are_anded_and_matchers_are_ored() {
    let mut config = config();
    config.automations[0].cases[0].applications = vec![
        WindowMatcher {
            id: "specific".into(),
            title: Some(TextCondition::contains("League")),
            class: Some(TextCondition::equals("GameWindow")),
            exe: Some(TextCondition::equals(r"C:\Games\League.exe")),
        },
        matcher("fallback", "Fallback"),
    ];
    let wrong_executable = FocusedWindow {
        title: Some("League".into()),
        class: Some("GameWindow".into()),
        exe: Some(PathBuf::from(r"C:\Browser.exe")),
    };
    let game = FocusedWindow {
        title: Some("League".into()),
        class: Some("GameWindow".into()),
        exe: Some(PathBuf::from(r"C:\Games\League.exe")),
    };

    assert_eq!(
        config.evaluate_window(&wrong_executable)[0].action.report,
        [0x86]
    );
    assert_eq!(config.evaluate_window(&game)[0].action.report, [0x87]);
}

#[test]
fn matching_exception_skips_to_next_case() {
    let mut config = config();
    config.automations[0].cases[0]
        .exceptions
        .push(matcher("browser", "Browser"));
    let window = FocusedWindow {
        title: Some("Browser Game".into()),
        ..FocusedWindow::default()
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
    let window = FocusedWindow {
        title: Some("Game".into()),
        ..FocusedWindow::default()
    };

    assert_eq!(config.evaluate_window(&window).len(), 2);
}
