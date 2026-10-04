use std::path::PathBuf;

use super::*;
use crate::config::{
    Automation, AutomationCase, MatchOperator, SendAction, TextCondition, WindowMatcher,
};

#[path = "behavior_tests.rs"]
mod behavior;

fn device(id: &str, name: &str, vid: u16) -> Device {
    Device {
        id: id.into(),
        name: name.into(),
        vid,
        pid: 0x002a,
        usage_page: 0xff60,
        usage: 0x61,
        report_length: 32,
        report_id: 0,
    }
}

fn condition(operator: MatchOperator, value: &str, case_sensitive: bool) -> TextCondition {
    TextCondition {
        operator,
        value: value.into(),
        case_sensitive,
    }
}

fn matcher(
    id: &str,
    title: Option<TextCondition>,
    class: Option<TextCondition>,
    exe: Option<TextCondition>,
) -> WindowMatcher {
    WindowMatcher {
        id: id.into(),
        title,
        class,
        exe,
    }
}

fn action(id: &str, label: &str, report: u8, device_ids: &[&str]) -> SendAction {
    SendAction {
        id: id.into(),
        label: label.into(),
        report: vec![report],
        device_ids: device_ids.iter().map(|id| (*id).into()).collect(),
    }
}

fn ordered_config() -> EditableConfig {
    EditableConfig {
        devices: vec![
            device("keyboard", "Keyboard", 0x1111),
            device("keypad", "Keypad", 0x2222),
        ],
        automations: vec![
            Automation {
                id: "layers".into(),
                name: "Layers".into(),
                enabled: true,
                cases: vec![
                    AutomationCase {
                        id: "game".into(),
                        name: "Game".into(),
                        applications: vec![
                            matcher(
                                "game-window",
                                Some(condition(MatchOperator::Contains, "league", false)),
                                Some(condition(MatchOperator::Regex, "^GameWindow$", true)),
                                None,
                            ),
                            matcher(
                                "game-executable",
                                None,
                                None,
                                Some(condition(
                                    MatchOperator::Equals,
                                    r"C:\Games\League.exe",
                                    false,
                                )),
                            ),
                        ],
                        exceptions: vec![matcher(
                            "launcher",
                            Some(condition(MatchOperator::Regex, "launcher$", false)),
                            None,
                            None,
                        )],
                        actions: vec![
                            action("layer", "Set layer", 0x87, &["keypad", "keyboard"]),
                            action("lighting", "Set lighting", 0x20, &["keyboard"]),
                        ],
                    },
                    AutomationCase {
                        id: "later-game".into(),
                        name: "Later game".into(),
                        applications: vec![matcher(
                            "any-game",
                            Some(TextCondition::contains("League")),
                            None,
                            None,
                        )],
                        actions: vec![action("later-action", "Must not win", 0xff, &["keyboard"])],
                        ..AutomationCase::default()
                    },
                ],
                otherwise_actions: vec![action("base", "Set base layer", 0x86, &["keyboard"])],
                ..Automation::default()
            },
            Automation {
                id: "status".into(),
                name: "Status".into(),
                enabled: true,
                cases: vec![AutomationCase {
                    id: "editor".into(),
                    name: "Editor".into(),
                    applications: vec![matcher(
                        "editor-window",
                        Some(condition(MatchOperator::Equals, "Editor", true)),
                        None,
                        None,
                    )],
                    actions: vec![action("status-on", "Status on", 0x01, &["keypad"])],
                    ..AutomationCase::default()
                }],
                otherwise_actions: vec![action("status-off", "Status off", 0x00, &["keypad"])],
                ..Automation::default()
            },
            Automation {
                id: "disabled".into(),
                name: "Disabled".into(),
                enabled: false,
                ..Automation::default()
            },
        ],
        ..EditableConfig::default()
    }
}

#[derive(Debug, PartialEq, Eq)]
struct DispatchSnapshot {
    automation: String,
    case: String,
    report: Vec<u8>,
    destinations: Vec<String>,
}

fn compiled_snapshots(config: &CompiledConfig, window: &FocusedWindow) -> Vec<DispatchSnapshot> {
    config
        .evaluate_event(&Event::FocusedWindowChanged {
            window: window.clone(),
            generation: 1,
        })
        .into_iter()
        .map(|dispatch| DispatchSnapshot {
            automation: dispatch.automation_name().into(),
            case: dispatch.case_name().into(),
            report: dispatch.report().into(),
            destinations: dispatch
                .destinations()
                .iter()
                .map(|device| device.id.clone())
                .collect(),
        })
        .collect()
}

#[test]
fn compiled_evaluation_preserves_all_declared_orders_and_exception_fallthrough() {
    let mut editable = ordered_config();
    editable.automations[0].cases[0].actions[0]
        .report
        .push(0xa1);
    let compiled = CompiledConfig::compile(&editable).unwrap();
    let windows = [
        FocusedWindow {
            title: Some("LEAGUE".into()),
            class: Some("GameWindow".into()),
            exe: None,
        },
        FocusedWindow {
            title: Some("League launcher".into()),
            class: Some("GameWindow".into()),
            exe: Some(PathBuf::from(r"C:\Games\League.exe")),
        },
        FocusedWindow {
            title: Some("Unrelated".into()),
            exe: Some(PathBuf::from(r"c:\games\league.exe")),
            ..FocusedWindow::default()
        },
        FocusedWindow {
            title: Some("Editor".into()),
            ..FocusedWindow::default()
        },
        FocusedWindow::default(),
    ];

    let expected = [
        vec![
            (
                "Layers",
                "Game",
                vec![0x87, 0xa1],
                vec!["keypad", "keyboard"],
            ),
            ("Layers", "Game", vec![0x20], vec!["keyboard"]),
            ("Status", "Otherwise", vec![0], vec!["keypad"]),
        ],
        vec![
            ("Layers", "Later game", vec![0xff], vec!["keyboard"]),
            ("Status", "Otherwise", vec![0], vec!["keypad"]),
        ],
        vec![
            (
                "Layers",
                "Game",
                vec![0x87, 0xa1],
                vec!["keypad", "keyboard"],
            ),
            ("Layers", "Game", vec![0x20], vec!["keyboard"]),
            ("Status", "Otherwise", vec![0], vec!["keypad"]),
        ],
        vec![
            ("Layers", "Otherwise", vec![0x86], vec!["keyboard"]),
            ("Status", "Editor", vec![1], vec!["keypad"]),
        ],
        vec![
            ("Layers", "Otherwise", vec![0x86], vec!["keyboard"]),
            ("Status", "Otherwise", vec![0], vec!["keypad"]),
        ],
    ];
    for (window, expected) in windows.iter().zip(expected) {
        let snapshots = compiled_snapshots(&compiled, window);
        let actual = snapshots
            .iter()
            .map(|dispatch| {
                (
                    dispatch.automation.as_str(),
                    dispatch.case.as_str(),
                    dispatch.report.clone(),
                    dispatch
                        .destinations
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }

    let game = compiled_snapshots(&compiled, &windows[0]);
    assert_eq!(
        game.iter()
            .map(|dispatch| dispatch.automation.as_str())
            .collect::<Vec<_>>(),
        ["Layers", "Layers", "Status"]
    );
    assert_eq!(game[0].case, "Game");
    assert_eq!(game[0].destinations, ["keypad", "keyboard"]);
}

#[test]
fn compiled_config_owns_dispatch_data_after_source_is_dropped() {
    let compiled = {
        let editable = ordered_config();
        CompiledConfig::compile(&editable).unwrap()
    };
    let window = FocusedWindow {
        title: Some("LEAGUE".into()),
        class: Some("GameWindow".into()),
        ..FocusedWindow::default()
    };

    let dispatches = compiled.evaluate_event(&Event::FocusedWindowChanged {
        window,
        generation: 1,
    });

    assert_eq!(dispatches[0].automation_name(), "Layers");
    assert_eq!(dispatches[0].case_name(), "Game");
    assert_eq!(dispatches[0].report(), [0x87]);
    assert_eq!(dispatches[0].destinations()[0].name, "Keypad");
}

#[test]
fn disabled_automation_with_dispatchable_actions_produces_nothing() {
    let editable = EditableConfig {
        devices: vec![device("keyboard", "Keyboard", 0x1234)],
        automations: vec![Automation {
            id: "disabled".into(),
            name: "Disabled".into(),
            enabled: false,
            cases: vec![AutomationCase {
                id: "matching".into(),
                name: "Matching".into(),
                applications: vec![matcher(
                    "game",
                    Some(TextCondition::contains("Game")),
                    None,
                    None,
                )],
                actions: vec![action("case-action", "Case action", 0x42, &["keyboard"])],
                ..AutomationCase::default()
            }],
            otherwise_actions: vec![action(
                "otherwise-action",
                "Otherwise action",
                0x43,
                &["keyboard"],
            )],
            ..Automation::default()
        }],
        ..EditableConfig::default()
    };
    let compiled = CompiledConfig::compile(&editable).unwrap();
    let window = FocusedWindow {
        title: Some("Game".into()),
        ..FocusedWindow::default()
    };

    assert!(
        compiled
            .evaluate_event(&Event::FocusedWindowChanged {
                window,
                generation: 1
            })
            .is_empty()
    );
}

#[test]
fn incomplete_disabled_draft_compiles_and_produces_nothing() {
    let editable = EditableConfig {
        automations: vec![Automation {
            id: "draft".into(),
            name: "Draft".into(),
            enabled: false,
            cases: vec![AutomationCase {
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
            }],
            ..Automation::default()
        }],
        ..EditableConfig::default()
    };

    let compiled = CompiledConfig::compile(&editable).unwrap();

    assert!(
        compiled
            .evaluate_event(&Event::FocusedWindowChanged {
                window: FocusedWindow::default(),
                generation: 1
            })
            .is_empty()
    );
}

#[test]
fn alias_destinations_retain_distinct_framing_in_declared_order() {
    let mut primary = device("primary", "Primary", 0x1234);
    primary.report_id = 1;
    primary.report_length = 16;
    let mut alias = primary.clone();
    alias.id = "alias".into();
    alias.name = "Alias".into();
    alias.report_id = 2;
    alias.report_length = 32;
    let editable = EditableConfig {
        devices: vec![primary, alias],
        automations: vec![Automation {
            id: "aliases".into(),
            name: "Aliases".into(),
            enabled: true,
            otherwise_actions: vec![action(
                "send-both",
                "Send both",
                0x42,
                &["alias", "primary"],
            )],
            ..Automation::default()
        }],
        ..EditableConfig::default()
    };

    let compiled = CompiledConfig::compile(&editable).unwrap();
    let dispatches = compiled.evaluate_event(&Event::FocusedWindowChanged {
        window: FocusedWindow::default(),
        generation: 1,
    });
    let destinations = dispatches[0].destinations();

    assert_eq!(destinations.len(), 2);
    assert_eq!(destinations[0].id, "alias");
    assert_eq!(destinations[1].id, "primary");
    assert_eq!(destinations[0].vid, destinations[1].vid);
    assert_eq!(destinations[0].pid, destinations[1].pid);
    assert_eq!(destinations[0].usage_page, destinations[1].usage_page);
    assert_eq!(destinations[0].usage, destinations[1].usage);
    assert_eq!(destinations[0].report_id, 2);
    assert_eq!(destinations[0].report_length, 32);
    assert_eq!(destinations[1].report_id, 1);
    assert_eq!(destinations[1].report_length, 16);
}

#[test]
fn public_compilation_rejects_validation_failures() {
    let mut invalid_regex = ordered_config();
    invalid_regex.automations[0].cases[0].applications[0].class =
        Some(condition(MatchOperator::Regex, "[", false));
    let regex_errors = CompiledConfig::compile(&invalid_regex).unwrap_err();
    assert!(
        regex_errors
            .iter()
            .any(|error| error.message == "invalid regular expression")
    );

    let mut unresolved = ordered_config();
    unresolved.automations[0].cases[0].actions[0].device_ids = vec!["missing".into()];
    let destination_errors = CompiledConfig::compile(&unresolved).unwrap_err();
    assert!(
        destination_errors
            .iter()
            .any(|error| error.message.contains("unknown device 'missing'"))
    );
}
