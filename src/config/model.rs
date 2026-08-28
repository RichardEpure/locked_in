use std::collections::HashSet;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditableConfig {
    pub settings: Settings,
    pub devices: Vec<Device>,
    pub automations: Vec<Automation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    #[serde(default = "default_true")]
    pub start_minimized: bool,
    #[serde(default = "default_true")]
    pub close_to_tray: bool,
    #[serde(default)]
    pub start_with_windows: bool,
    #[serde(default)]
    pub log_level: LogLevel,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            start_minimized: true,
            close_to_tray: true,
            start_with_windows: false,
            log_level: LogLevel::Info,
        }
    }
}

const fn default_true() -> bool {
    true
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Error,
    #[default]
    Info,
    Debug,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub vid: u16,
    pub pid: u16,
    pub usage_page: u16,
    pub usage: u16,
    pub report_length: u16,
    pub report_id: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Automation {
    pub id: String,
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub event: Event,
    #[serde(default)]
    pub cases: Vec<AutomationCase>,
    #[serde(default)]
    pub otherwise_actions: Vec<SendAction>,
}

impl Default for Automation {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: "New automation".to_string(),
            enabled: false,
            event: Event::default(),
            cases: Vec::new(),
            otherwise_actions: Vec::new(),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    #[default]
    FocusedWindowChanged,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AutomationCase {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub applications: Vec<WindowMatcher>,
    #[serde(default)]
    pub exceptions: Vec<WindowMatcher>,
    #[serde(default)]
    pub actions: Vec<SendAction>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WindowMatcher {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<TextCondition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<TextCondition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exe: Option<TextCondition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TextCondition {
    pub operator: MatchOperator,
    pub value: String,
    #[serde(default)]
    pub case_sensitive: bool,
}

impl TextCondition {
    pub fn contains(value: impl Into<String>) -> Self {
        Self {
            operator: MatchOperator::Contains,
            value: value.into(),
            case_sensitive: false,
        }
    }

    pub fn equals(value: impl Into<String>) -> Self {
        Self {
            operator: MatchOperator::Equals,
            value: value.into(),
            case_sensitive: false,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchOperator {
    Equals,
    #[default]
    Contains,
    Regex,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SendAction {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub report: Vec<u8>,
    #[serde(default)]
    pub device_ids: Vec<String>,
}

impl EditableConfig {
    pub fn next_id(&self, prefix: &str) -> String {
        let used = self
            .devices
            .iter()
            .map(|item| item.id.as_str())
            .chain(self.automations.iter().map(|item| item.id.as_str()))
            .collect::<HashSet<_>>();
        next_available_id(prefix, &used)
    }
}

fn next_available_id(prefix: &str, used: &HashSet<&str>) -> String {
    let prefix = prefix.trim().to_lowercase().replace(' ', "-");
    if !used.contains(prefix.as_str()) {
        return prefix;
    }
    (2..)
        .map(|suffix| format!("{prefix}-{suffix}"))
        .find(|candidate| !used.contains(candidate.as_str()))
        .expect("identifier search is finite")
}
