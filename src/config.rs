use std::{collections::HashSet, env, fs, io::Write, path::PathBuf};

use anyhow::{Context, Result};
use figment::{
    Figment,
    providers::{Format, Toml},
};
use regex::RegexBuilder;
use serde::{Deserialize, Serialize};

use crate::win::WindowMetadata;

const CONFIG_PATH: &str = "config.toml";

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u8,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub devices: Vec<Device>,
    #[serde(default)]
    pub automations: Vec<Automation>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: schema_version(),
            settings: Settings::default(),
            devices: Vec::new(),
            automations: Vec::new(),
        }
    }
}

const fn schema_version() -> u8 {
    2
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub path: String,
    pub message: String,
}

#[derive(Debug)]
pub struct EvaluatedAction<'a> {
    pub automation_name: &'a str,
    pub case_name: &'a str,
    pub action: &'a SendAction,
    pub devices: Vec<&'a Device>,
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = config_path()?;
        if !path.is_file() {
            let config = Self::default();
            config.save()?;
            return Ok(config);
        }

        let config = Figment::new()
            .merge(Toml::file(&path))
            .extract::<Self>()
            .with_context(|| format!("Failed to load {}", path.display()))?;

        if config.version != schema_version() {
            anyhow::bail!(
                "Unsupported config version {}; expected {}",
                config.version,
                schema_version()
            );
        }
        let errors = config.validate();
        if !errors.is_empty() {
            anyhow::bail!(format_validation_errors(&errors));
        }
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let errors = self.validate();
        if !errors.is_empty() {
            anyhow::bail!(format_validation_errors(&errors));
        }

        let path = config_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }

        let contents = toml::to_string_pretty(self).context("Failed to serialize config")?;
        let temporary = path.with_extension("toml.tmp");
        let mut file = fs::File::create(&temporary)
            .with_context(|| format!("Failed to create {}", temporary.display()))?;
        file.write_all(contents.as_bytes())
            .context("Failed to write temporary config")?;
        file.flush().context("Failed to flush temporary config")?;
        replace_file(&temporary, &path)?;
        Ok(())
    }

    pub fn validate(&self) -> Vec<ValidationError> {
        let mut errors = Vec::new();
        let mut device_ids = HashSet::new();
        for (index, device) in self.devices.iter().enumerate() {
            let path = format!("devices[{index}]");
            if device.id.trim().is_empty() {
                push_error(&mut errors, &path, "id is required");
            } else if !device_ids.insert(device.id.as_str()) {
                push_error(&mut errors, &path, "id must be unique");
            }
            if device.name.trim().is_empty() {
                push_error(&mut errors, &path, "name is required");
            }
            if device.report_length == 0 {
                push_error(
                    &mut errors,
                    &path,
                    "report length must be greater than zero",
                );
            }
        }

        let mut automation_ids = HashSet::new();
        for (automation_index, automation) in self.automations.iter().enumerate() {
            let path = format!("automations[{automation_index}]");
            if automation.id.trim().is_empty() {
                push_error(&mut errors, &path, "id is required");
            } else if !automation_ids.insert(automation.id.as_str()) {
                push_error(&mut errors, &path, "id must be unique");
            }
            if automation.name.trim().is_empty() {
                push_error(&mut errors, &path, "name is required");
            }
            if automation.enabled
                && automation.cases.is_empty()
                && automation.otherwise_actions.is_empty()
            {
                push_error(
                    &mut errors,
                    &path,
                    "enabled automation has no cases or otherwise actions",
                );
            }

            let mut case_ids = HashSet::new();
            let mut action_ids = HashSet::new();
            for (case_index, case) in automation.cases.iter().enumerate() {
                let case_path = format!("{path}.cases[{case_index}]");
                validate_id(&case.id, &case_path, &mut case_ids, &mut errors);
                if automation.enabled && case.applications.is_empty() {
                    push_error(
                        &mut errors,
                        &case_path,
                        "enabled case needs an application matcher",
                    );
                }
                if automation.enabled && case.actions.is_empty() {
                    push_error(&mut errors, &case_path, "enabled case needs an action");
                }
                validate_matchers(
                    &case.applications,
                    &format!("{case_path}.applications"),
                    automation.enabled,
                    &mut errors,
                );
                validate_matchers(
                    &case.exceptions,
                    &format!("{case_path}.exceptions"),
                    automation.enabled,
                    &mut errors,
                );
                validate_child_ids(
                    case.applications
                        .iter()
                        .chain(&case.exceptions)
                        .map(|matcher| matcher.id.as_str()),
                    &case_path,
                    "matcher",
                    &mut errors,
                );
                for action in &case.actions {
                    validate_id(&action.id, &case_path, &mut action_ids, &mut errors);
                }
                validate_actions(
                    &case.actions,
                    &format!("{case_path}.actions"),
                    automation.enabled,
                    &self.devices,
                    &mut errors,
                );
            }
            for action in &automation.otherwise_actions {
                validate_id(&action.id, &path, &mut action_ids, &mut errors);
            }
            validate_actions(
                &automation.otherwise_actions,
                &format!("{path}.otherwise_actions"),
                automation.enabled,
                &self.devices,
                &mut errors,
            );
        }
        errors
    }

    pub fn validate_action(&self, action: &SendAction) -> Vec<ValidationError> {
        let mut errors = Vec::new();
        validate_actions(
            std::slice::from_ref(action),
            "action",
            true,
            &self.devices,
            &mut errors,
        );
        errors
    }

    pub fn evaluate_window<'a>(&'a self, window: &WindowMetadata) -> Vec<EvaluatedAction<'a>> {
        let mut evaluated = Vec::new();
        for automation in self
            .automations
            .iter()
            .filter(|automation| automation.enabled)
        {
            let selected = automation.cases.iter().find(|case| {
                case.applications
                    .iter()
                    .any(|matcher| matcher.matches(window))
                    && !case
                        .exceptions
                        .iter()
                        .any(|matcher| matcher.matches(window))
            });
            let (case_name, actions) = if let Some(case) = selected {
                (case.name.as_str(), case.actions.as_slice())
            } else {
                ("Otherwise", automation.otherwise_actions.as_slice())
            };

            for action in actions {
                evaluated.push(EvaluatedAction {
                    automation_name: &automation.name,
                    case_name,
                    action,
                    devices: action
                        .device_ids
                        .iter()
                        .filter_map(|id| self.devices.iter().find(|device| device.id == *id))
                        .collect(),
                });
            }
        }
        evaluated
    }

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

impl WindowMatcher {
    fn matches(&self, window: &WindowMetadata) -> bool {
        matches_condition(self.title.as_ref(), window.title.as_deref())
            && matches_condition(self.class.as_ref(), window.class.as_deref())
            && matches_condition(
                self.exe.as_ref(),
                window.exe.as_ref().and_then(|value| value.to_str()),
            )
    }
}

fn matches_condition(condition: Option<&TextCondition>, actual: Option<&str>) -> bool {
    let Some(condition) = condition else {
        return true;
    };
    let Some(actual) = actual else {
        return false;
    };

    match condition.operator {
        MatchOperator::Equals => {
            if condition.case_sensitive {
                actual == condition.value
            } else {
                actual.eq_ignore_ascii_case(&condition.value)
            }
        }
        MatchOperator::Contains => {
            if condition.case_sensitive {
                actual.contains(&condition.value)
            } else {
                actual
                    .to_lowercase()
                    .contains(&condition.value.to_lowercase())
            }
        }
        MatchOperator::Regex => RegexBuilder::new(&condition.value)
            .case_insensitive(!condition.case_sensitive)
            .build()
            .is_ok_and(|regex| regex.is_match(actual)),
    }
}

fn validate_matchers(
    matchers: &[WindowMatcher],
    path: &str,
    require_complete: bool,
    errors: &mut Vec<ValidationError>,
) {
    for (index, matcher) in matchers.iter().enumerate() {
        let matcher_path = format!("{path}[{index}]");
        if require_complete
            && matcher.title.is_none()
            && matcher.class.is_none()
            && matcher.exe.is_none()
        {
            push_error(errors, &matcher_path, "at least one field is required");
        }
        for (field, condition) in [
            ("title", matcher.title.as_ref()),
            ("class", matcher.class.as_ref()),
            ("exe", matcher.exe.as_ref()),
        ] {
            let Some(condition) = condition else { continue };
            if require_complete && condition.value.is_empty() {
                push_error(
                    errors,
                    &format!("{matcher_path}.{field}"),
                    "value is required",
                );
            }
            if condition.operator == MatchOperator::Regex
                && RegexBuilder::new(&condition.value)
                    .case_insensitive(!condition.case_sensitive)
                    .build()
                    .is_err()
            {
                push_error(
                    errors,
                    &format!("{matcher_path}.{field}"),
                    "invalid regular expression",
                );
            }
        }
    }
}

fn validate_actions(
    actions: &[SendAction],
    path: &str,
    require_complete: bool,
    devices: &[Device],
    errors: &mut Vec<ValidationError>,
) {
    for (index, action) in actions.iter().enumerate() {
        let action_path = format!("{path}[{index}]");
        if require_complete && action.report.is_empty() {
            push_error(errors, &action_path, "report is required");
        }
        if require_complete && action.device_ids.is_empty() {
            push_error(errors, &action_path, "at least one destination is required");
        }
        let mut seen = HashSet::new();
        for device_id in &action.device_ids {
            if !seen.insert(device_id) {
                push_error(errors, &action_path, "destinations must not be duplicated");
            }
            let Some(device) = devices.iter().find(|device| device.id == *device_id) else {
                push_error(
                    errors,
                    &action_path,
                    &format!("unknown device '{device_id}'"),
                );
                continue;
            };
            if action.report.len() > device.report_length as usize {
                push_error(
                    errors,
                    &action_path,
                    &format!(
                        "report exceeds {} byte capacity of {}",
                        device.report_length, device.name
                    ),
                );
            }
        }
    }
}

fn push_error(errors: &mut Vec<ValidationError>, path: &str, message: &str) {
    errors.push(ValidationError {
        path: path.to_string(),
        message: message.to_string(),
    });
}

fn validate_id<'a>(
    id: &'a str,
    path: &str,
    seen: &mut HashSet<&'a str>,
    errors: &mut Vec<ValidationError>,
) {
    if id.trim().is_empty() {
        push_error(errors, path, "id is required");
    } else if !seen.insert(id) {
        push_error(errors, path, "id must be unique");
    }
}

fn validate_child_ids<'a>(
    ids: impl Iterator<Item = &'a str>,
    path: &str,
    kind: &str,
    errors: &mut Vec<ValidationError>,
) {
    let mut seen = HashSet::new();
    for id in ids {
        if id.trim().is_empty() {
            push_error(errors, path, &format!("{kind} id is required"));
        } else if !seen.insert(id) {
            push_error(errors, path, &format!("{kind} ids must be unique"));
        }
    }
}

fn format_validation_errors(errors: &[ValidationError]) -> String {
    errors
        .iter()
        .map(|error| format!("{}: {}", error.path, error.message))
        .collect::<Vec<_>>()
        .join("\n")
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

pub fn config_path() -> Result<PathBuf> {
    Ok(data_directory()?.join(CONFIG_PATH))
}

pub fn data_directory() -> Result<PathBuf> {
    if let Some(path) = env::var_os("LOCKED_IN_DATA_DIR") {
        return Ok(PathBuf::from(path));
    }
    if cfg!(debug_assertions) {
        return env::current_dir().context("Failed to get current directory");
    }
    env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|path| path.join("LockedIn"))
        .context("LOCALAPPDATA is unavailable")
}

#[cfg(windows)]
fn replace_file(source: &std::path::Path, destination: &std::path::Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::Storage::FileSystem::{
            MOVE_FILE_FLAGS, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        },
        core::PCWSTR,
    };

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    unsafe {
        MoveFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(destination.as_ptr()),
            MOVE_FILE_FLAGS(MOVEFILE_REPLACE_EXISTING.0 | MOVEFILE_WRITE_THROUGH.0),
        )
        .with_context(|| "Failed to atomically replace configuration")?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(source: &std::path::Path, destination: &std::path::Path) -> Result<()> {
    fs::rename(source, destination)
        .with_context(|| format!("Failed to replace {}", destination.display()))
}

#[cfg(test)]
mod tests;
