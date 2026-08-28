use std::path::PathBuf;

use super::*;
use crate::config::{Automation, AutomationCase};

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

fn parity_config() -> EditableConfig {
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
    label: String,
    report: Vec<u8>,
    destinations: Vec<String>,
}

fn editable_snapshots(config: &EditableConfig, window: &FocusedWindow) -> Vec<DispatchSnapshot> {
    config
        .evaluate_window(window)
        .into_iter()
        .map(|dispatch| DispatchSnapshot {
            automation: dispatch.automation_name.into(),
            case: dispatch.case_name.into(),
            label: dispatch.action.label.clone(),
            report: dispatch.action.report.clone(),
            destinations: dispatch
                .devices
                .iter()
                .map(|device| device.id.clone())
                .collect(),
        })
        .collect()
}

fn active_snapshots(config: &ActiveConfig, window: &FocusedWindow) -> Vec<DispatchSnapshot> {
    config
        .evaluate_window(window)
        .into_iter()
        .map(|dispatch| DispatchSnapshot {
            automation: dispatch.automation_name().into(),
            case: dispatch.case_name().into(),
            label: dispatch.action_label().into(),
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
fn active_evaluation_preserves_editable_semantics_and_all_declared_orders() {
    let editable = parity_config();
    let active = ActiveConfig::compile(&editable).unwrap();
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

    for window in &windows {
        assert_eq!(
            active_snapshots(&active, window),
            editable_snapshots(&editable, window)
        );
    }

    let game = active_snapshots(&active, &windows[0]);
    assert_eq!(
        game.iter()
            .map(|dispatch| dispatch.automation.as_str())
            .collect::<Vec<_>>(),
        ["Layers", "Layers", "Status"]
    );
    assert_eq!(game[0].case, "Game");
    assert_eq!(game[0].label, "Set layer");
    assert_eq!(game[1].label, "Set lighting");
    assert_eq!(game[0].destinations, ["keypad", "keyboard"]);
}

#[test]
fn compiled_config_owns_dispatch_data_after_source_is_dropped() {
    let active = {
        let editable = parity_config();
        ActiveConfig::compile(&editable).unwrap()
    };
    let window = FocusedWindow {
        title: Some("LEAGUE".into()),
        class: Some("GameWindow".into()),
        ..FocusedWindow::default()
    };

    let dispatches = active.evaluate_window(&window);

    assert_eq!(dispatches[0].automation_name(), "Layers");
    assert_eq!(dispatches[0].case_name(), "Game");
    assert_eq!(dispatches[0].action_label(), "Set layer");
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
    let active = ActiveConfig::compile(&editable).unwrap();
    let window = FocusedWindow {
        title: Some("Game".into()),
        ..FocusedWindow::default()
    };

    assert!(active.evaluate_window(&window).is_empty());
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

    let active = ActiveConfig::compile(&editable).unwrap();

    assert!(active.evaluate_window(&FocusedWindow::default()).is_empty());
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

    let active = ActiveConfig::compile(&editable).unwrap();
    let dispatches = active.evaluate_window(&FocusedWindow::default());
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
fn case_insensitive_contains_preserves_lowercase_matching_semantics() {
    for (actual, value) in [
        ("Prefix LEAGUE suffix", "league"),
        ("No match", "league"),
        ("İSTANBUL", "İS"),
        ("CAFÉ", "fé"),
        ("anything", ""),
    ] {
        let condition = condition(MatchOperator::Contains, value, false);
        let compiled = compile_condition(Some(&condition), "matcher.title")
            .unwrap()
            .unwrap();

        assert_eq!(
            matches_condition(Some(&compiled), Some(actual)),
            actual.to_lowercase().contains(&value.to_lowercase()),
            "actual={actual:?}, value={value:?}"
        );
    }
}

#[test]
fn public_compilation_rejects_validation_failures() {
    let mut invalid_regex = parity_config();
    invalid_regex.automations[0].cases[0].applications[0].class =
        Some(condition(MatchOperator::Regex, "[", false));
    let regex_errors = ActiveConfig::compile(&invalid_regex).unwrap_err();
    assert!(
        regex_errors
            .iter()
            .any(|error| error.message == "invalid regular expression")
    );

    let mut unresolved = parity_config();
    unresolved.automations[0].cases[0].actions[0].device_ids = vec!["missing".into()];
    let destination_errors = ActiveConfig::compile(&unresolved).unwrap_err();
    assert!(
        destination_errors
            .iter()
            .any(|error| error.message.contains("unknown device 'missing'"))
    );
}

#[test]
fn compilation_steps_return_errors_instead_of_dropping_invalid_data() {
    let invalid_regex = condition(MatchOperator::Regex, "[", false);
    let regex_error = compile_condition(Some(&invalid_regex), "matcher.title").unwrap_err();
    assert_eq!(regex_error.path, "matcher.title");

    let unresolved = action("send", "Send", 0x42, &["missing"]);
    let destination_error = compile_actions(&[unresolved], &HashMap::new(), "actions").unwrap_err();
    assert_eq!(destination_error.path, "actions[0]");
    assert!(destination_error.message.contains("device 'missing'"));
}
