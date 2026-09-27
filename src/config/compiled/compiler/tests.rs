use super::super::matches_condition;
use super::*;

#[test]
fn case_insensitive_contains_preserves_lowercase_matching_semantics() {
    for (actual, value, expected) in [
        ("Prefix LEAGUE suffix", "league", true),
        ("No match", "league", false),
        ("İSTANBUL", "İS", true),
        ("CAFÉ", "fé", true),
        ("anything", "", true),
        ("", "", true),
    ] {
        let condition = TextCondition {
            operator: MatchOperator::Contains,
            value: value.into(),
            case_sensitive: false,
        };
        let compiled = compile_condition(Some(&condition), "matcher.title")
            .unwrap()
            .unwrap();

        assert_eq!(
            matches_condition(Some(&compiled), Some(actual)),
            expected,
            "actual={actual:?}, value={value:?}"
        );
    }
}

#[test]
fn equality_folds_only_ascii_and_missing_metadata_never_matches_a_condition() {
    for (actual, value, expected) in [("EDITOR", "editor", true), ("CAFÉ", "café", false)] {
        let condition = TextCondition {
            operator: MatchOperator::Equals,
            value: value.into(),
            case_sensitive: false,
        };
        let compiled = compile_condition(Some(&condition), "title").unwrap();
        assert_eq!(matches_condition(compiled.as_ref(), Some(actual)), expected);
    }
    let empty = TextCondition {
        operator: MatchOperator::Contains,
        value: String::new(),
        case_sensitive: false,
    };
    let compiled = compile_condition(Some(&empty), "title").unwrap();
    assert!(!matches_condition(compiled.as_ref(), None));
    assert!(matches_condition(None, None));
}

#[test]
fn compilation_steps_return_errors_instead_of_dropping_invalid_data() {
    let invalid_regex = TextCondition {
        operator: MatchOperator::Regex,
        value: "[".into(),
        case_sensitive: false,
    };
    let regex_error = compile_condition(Some(&invalid_regex), "matcher.title").unwrap_err();
    assert_eq!(regex_error.path, "matcher.title");

    let unresolved = SendAction {
        id: "send".into(),
        label: "Send".into(),
        report: vec![0x42],
        device_ids: vec!["missing".into()],
    };
    let errors = EditableConfig::default().validate_action(&unresolved);
    assert_eq!(errors[0].path, "action[0]");
    assert!(errors[0].message.contains("device 'missing'"));
}
