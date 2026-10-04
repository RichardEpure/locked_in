use super::*;

#[test]
fn typed_edits_keep_fields_independent_and_preserve_operator_and_case() {
    let mut matcher = WindowMatcher::default();
    MatcherField::Title.apply(&mut matcher, ConditionEdit::Value("Title".into()));
    MatcherField::Class.apply(&mut matcher, ConditionEdit::Value("Class".into()));
    MatcherField::Executable.apply(&mut matcher, ConditionEdit::Value("app.exe".into()));
    assert_eq!(
        matcher.title.as_ref().unwrap().operator,
        MatchOperator::Contains
    );
    assert_eq!(
        matcher.class.as_ref().unwrap().operator,
        MatchOperator::Contains
    );
    assert_eq!(
        matcher.exe.as_ref().unwrap().operator,
        MatchOperator::Equals
    );
    let original = matcher.clone();
    for field in [
        MatcherField::Title,
        MatcherField::Class,
        MatcherField::Executable,
    ] {
        let mut changed = original.clone();
        field.apply(&mut changed, ConditionEdit::Operator(MatchOperator::Regex));
        field.apply(&mut changed, ConditionEdit::CaseSensitive(true));
        field.apply(&mut changed, ConditionEdit::Value("Updated".into()));
        let condition = field.slot(&mut changed).as_ref().unwrap();
        assert_eq!(condition.value, "Updated");
        assert_eq!(condition.operator, MatchOperator::Regex);
        assert!(condition.case_sensitive);
        *field.slot(&mut changed) = field.slot(&mut original.clone()).clone();
        assert_eq!(changed, original);
    }
}

#[test]
fn empty_value_removes_condition_and_discards_operator_and_case_choices() {
    for field in [
        MatcherField::Title,
        MatcherField::Class,
        MatcherField::Executable,
    ] {
        let mut matcher = WindowMatcher::default();
        field.apply(&mut matcher, ConditionEdit::Operator(MatchOperator::Regex));
        field.apply(&mut matcher, ConditionEdit::CaseSensitive(true));
        assert!(field.slot(&mut matcher).is_none());
        field.apply(&mut matcher, ConditionEdit::Value(" ".into()));
        let condition = field.slot(&mut matcher).as_ref().unwrap();
        assert_eq!(condition.operator, field.default_operator());
        assert!(!condition.case_sensitive);
        assert_eq!(condition.value, " ");
        field.apply(&mut matcher, ConditionEdit::Value(String::new()));
        assert!(field.slot(&mut matcher).is_none());
    }
}
