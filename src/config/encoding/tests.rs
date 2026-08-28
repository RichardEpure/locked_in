use super::*;

#[test]
fn encoding_always_emits_explicit_schema_version_two() {
    let encoded = encode(&EditableConfig::default()).unwrap();
    let document = toml::from_str::<toml::Table>(&encoded).unwrap();

    assert_eq!(encoded.lines().next(), Some("version = 2"));
    assert_eq!(document.get("version"), Some(&toml::Value::Integer(2)));
    assert_eq!(document.len(), 4);
    for field in ["version", "settings", "devices", "automations"] {
        assert!(document.contains_key(field));
    }
}

#[test]
fn schema_version_is_not_editable_configuration_data() {
    let mut config = EditableConfig::default();
    config.settings.start_minimized = false;

    let decoded = decode(&encode(&config).unwrap()).unwrap();

    assert_eq!(decoded, config);
}

#[test]
fn missing_schema_version_is_rejected() {
    assert!(decode("[settings]\nstart_minimized = true").is_err());
}

#[test]
fn legacy_and_future_schema_versions_are_rejected() {
    for version in [1, 3] {
        let error = decode(&format!("version = {version}")).unwrap_err();
        assert!(error.to_string().contains("unsupported config version"));
    }
}

#[test]
fn legacy_and_unknown_fields_are_rejected() {
    assert!(decode("version = 2\n[[rules]]\nname = \"Legacy rule\"").is_err());
    assert!(decode("version = 2\nextra = true").is_err());
}

#[test]
fn unknown_settings_fields_are_rejected() {
    let encoded = r#"
        version = 2

        [settings]
        close_to_try = true
    "#;

    assert!(
        decode(encoded)
            .unwrap_err()
            .to_string()
            .contains("unknown field")
    );
}

#[test]
fn unknown_fields_are_rejected_for_every_nested_record() {
    let records = [
        (
            "Device",
            r#"
                [[devices]]
                id = "keyboard"
                name = "Keyboard"
                vid = 1
                pid = 2
                usage_page = 3
                usage = 4
                report_length = 32
                report_id = 0
                unexpected = true
            "#,
        ),
        (
            "Automation",
            r#"
                [[automations]]
                id = "layers"
                name = "Layers"
                unexpected = true
            "#,
        ),
        (
            "AutomationCase",
            r#"
                [[automations]]
                id = "layers"
                name = "Layers"

                [[automations.cases]]
                id = "game"
                unexpected = true
            "#,
        ),
        (
            "WindowMatcher",
            r#"
                [[automations]]
                id = "layers"
                name = "Layers"

                [[automations.cases]]
                id = "game"

                [[automations.cases.applications]]
                id = "game-window"
                unexpected = true
            "#,
        ),
        (
            "TextCondition",
            r#"
                [[automations]]
                id = "layers"
                name = "Layers"

                [[automations.cases]]
                id = "game"

                [[automations.cases.applications]]
                id = "game-window"

                [automations.cases.applications.title]
                operator = "contains"
                value = "Game"
                unexpected = true
            "#,
        ),
        (
            "SendAction",
            r#"
                [[automations]]
                id = "layers"
                name = "Layers"

                [[automations.cases]]
                id = "game"

                [[automations.cases.actions]]
                id = "switch-layer"
                unexpected = true
            "#,
        ),
    ];

    for (record, body) in records {
        let error = decode(&format!("version = 2\n{body}")).unwrap_err();
        assert!(
            error.to_string().contains("unknown field"),
            "{record} accepted an unknown field: {error}"
        );
    }
}
