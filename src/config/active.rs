use std::collections::HashMap;

use regex::{Regex, RegexBuilder};

use super::{
    Device, EditableConfig, MatchOperator, SendAction, TextCondition, ValidationError,
    WindowMatcher,
};
use crate::focused_window::FocusedWindow;

#[derive(Debug, Clone)]
pub struct ActiveConfig {
    automations: Box<[ActiveAutomation]>,
}

#[derive(Debug)]
pub struct ActiveDispatch<'a> {
    automation_name: &'a str,
    case_name: &'a str,
    action_label: &'a str,
    report: &'a [u8],
    destinations: &'a [Device],
}

#[derive(Debug, Clone)]
struct ActiveAutomation {
    name: String,
    cases: Box<[ActiveCase]>,
    otherwise_actions: Box<[ActiveAction]>,
}

#[derive(Debug, Clone)]
struct ActiveCase {
    name: String,
    applications: Box<[ActiveWindowMatcher]>,
    exceptions: Box<[ActiveWindowMatcher]>,
    actions: Box<[ActiveAction]>,
}

#[derive(Debug, Clone)]
struct ActiveWindowMatcher {
    title: Option<ActiveTextCondition>,
    class: Option<ActiveTextCondition>,
    exe: Option<ActiveTextCondition>,
}

#[derive(Debug, Clone)]
enum ActiveTextCondition {
    Equals { value: String, case_sensitive: bool },
    Contains { value: String, case_sensitive: bool },
    Regex(Regex),
}

#[derive(Debug, Clone)]
struct ActiveAction {
    label: String,
    report: Box<[u8]>,
    destinations: Box<[Device]>,
}

impl ActiveConfig {
    pub fn compile(editable: &EditableConfig) -> Result<Self, Vec<ValidationError>> {
        let validation_errors = editable.validate();
        if !validation_errors.is_empty() {
            return Err(validation_errors);
        }

        let devices = editable
            .devices
            .iter()
            .map(|device| (device.id.as_str(), device))
            .collect::<HashMap<_, _>>();
        let automations = editable
            .automations
            .iter()
            .enumerate()
            .filter(|(_, automation)| automation.enabled)
            .map(|(index, automation)| ActiveAutomation::compile(automation, &devices, index))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| vec![error])?;

        Ok(Self {
            automations: automations.into_boxed_slice(),
        })
    }

    pub fn evaluate_window<'a>(&'a self, window: &FocusedWindow) -> Vec<ActiveDispatch<'a>> {
        let mut dispatches = Vec::new();
        for automation in &self.automations {
            let selected = automation.cases.iter().find(|case| case.matches(window));
            let (case_name, actions) = if let Some(case) = selected {
                (case.name.as_str(), case.actions.as_ref())
            } else {
                ("Otherwise", automation.otherwise_actions.as_ref())
            };

            dispatches.extend(actions.iter().map(|action| ActiveDispatch {
                automation_name: &automation.name,
                case_name,
                action_label: &action.label,
                report: &action.report,
                destinations: &action.destinations,
            }));
        }
        dispatches
    }
}

impl<'a> ActiveDispatch<'a> {
    pub fn automation_name(&self) -> &'a str {
        self.automation_name
    }

    pub fn case_name(&self) -> &'a str {
        self.case_name
    }

    pub fn action_label(&self) -> &'a str {
        self.action_label
    }

    pub fn report(&self) -> &'a [u8] {
        self.report
    }

    pub fn destinations(&self) -> &'a [Device] {
        self.destinations
    }
}

impl ActiveAutomation {
    fn compile(
        automation: &super::Automation,
        devices: &HashMap<&str, &Device>,
        automation_index: usize,
    ) -> Result<Self, ValidationError> {
        let path = format!("automations[{automation_index}]");
        let cases = automation
            .cases
            .iter()
            .enumerate()
            .map(|(index, case)| ActiveCase::compile(case, devices, &path, index))
            .collect::<Result<Vec<_>, _>>()?;
        let otherwise_actions = compile_actions(
            &automation.otherwise_actions,
            devices,
            &format!("{path}.otherwise_actions"),
        )?;

        Ok(Self {
            name: automation.name.clone(),
            cases: cases.into_boxed_slice(),
            otherwise_actions,
        })
    }
}

impl ActiveCase {
    fn compile(
        case: &super::AutomationCase,
        devices: &HashMap<&str, &Device>,
        automation_path: &str,
        case_index: usize,
    ) -> Result<Self, ValidationError> {
        let path = format!("{automation_path}.cases[{case_index}]");
        Ok(Self {
            name: case.name.clone(),
            applications: compile_matchers(&case.applications, &format!("{path}.applications"))?,
            exceptions: compile_matchers(&case.exceptions, &format!("{path}.exceptions"))?,
            actions: compile_actions(&case.actions, devices, &format!("{path}.actions"))?,
        })
    }

    fn matches(&self, window: &FocusedWindow) -> bool {
        self.applications
            .iter()
            .any(|matcher| matcher.matches(window))
            && !self
                .exceptions
                .iter()
                .any(|matcher| matcher.matches(window))
    }
}

impl ActiveWindowMatcher {
    fn compile(matcher: &WindowMatcher, path: &str) -> Result<Self, ValidationError> {
        Ok(Self {
            title: compile_condition(matcher.title.as_ref(), &format!("{path}.title"))?,
            class: compile_condition(matcher.class.as_ref(), &format!("{path}.class"))?,
            exe: compile_condition(matcher.exe.as_ref(), &format!("{path}.exe"))?,
        })
    }

    fn matches(&self, window: &FocusedWindow) -> bool {
        matches_condition(self.title.as_ref(), window.title.as_deref())
            && matches_condition(self.class.as_ref(), window.class.as_deref())
            && matches_condition(
                self.exe.as_ref(),
                window.exe.as_ref().and_then(|value| value.to_str()),
            )
    }
}

fn compile_matchers(
    matchers: &[WindowMatcher],
    path: &str,
) -> Result<Box<[ActiveWindowMatcher]>, ValidationError> {
    matchers
        .iter()
        .enumerate()
        .map(|(index, matcher)| ActiveWindowMatcher::compile(matcher, &format!("{path}[{index}]")))
        .collect::<Result<Vec<_>, _>>()
        .map(Vec::into_boxed_slice)
}

fn compile_condition(
    condition: Option<&TextCondition>,
    path: &str,
) -> Result<Option<ActiveTextCondition>, ValidationError> {
    let Some(condition) = condition else {
        return Ok(None);
    };
    let compiled = match condition.operator {
        MatchOperator::Equals => ActiveTextCondition::Equals {
            value: condition.value.clone(),
            case_sensitive: condition.case_sensitive,
        },
        MatchOperator::Contains => ActiveTextCondition::Contains {
            value: if condition.case_sensitive {
                condition.value.clone()
            } else {
                condition.value.to_lowercase()
            },
            case_sensitive: condition.case_sensitive,
        },
        MatchOperator::Regex => ActiveTextCondition::Regex(
            RegexBuilder::new(&condition.value)
                .case_insensitive(!condition.case_sensitive)
                .build()
                .map_err(|error| ValidationError {
                    path: path.to_string(),
                    message: format!("failed to compile regular expression: {error}"),
                })?,
        ),
    };
    Ok(Some(compiled))
}

fn compile_actions(
    actions: &[SendAction],
    devices: &HashMap<&str, &Device>,
    path: &str,
) -> Result<Box<[ActiveAction]>, ValidationError> {
    actions
        .iter()
        .enumerate()
        .map(|(index, action)| {
            let action_path = format!("{path}[{index}]");
            let destinations = action
                .device_ids
                .iter()
                .map(|device_id| {
                    devices
                        .get(device_id.as_str())
                        .copied()
                        .cloned()
                        .ok_or_else(|| ValidationError {
                            path: action_path.clone(),
                            message: format!("failed to resolve device '{device_id}'"),
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(ActiveAction {
                label: action.label.clone(),
                report: action.report.clone().into_boxed_slice(),
                destinations: destinations.into_boxed_slice(),
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Vec::into_boxed_slice)
}

fn matches_condition(condition: Option<&ActiveTextCondition>, actual: Option<&str>) -> bool {
    let Some(condition) = condition else {
        return true;
    };
    let Some(actual) = actual else {
        return false;
    };

    match condition {
        ActiveTextCondition::Equals {
            value,
            case_sensitive,
        } => {
            if *case_sensitive {
                actual == value
            } else {
                actual.eq_ignore_ascii_case(value)
            }
        }
        ActiveTextCondition::Contains {
            value,
            case_sensitive,
        } => {
            if *case_sensitive {
                actual.contains(value)
            } else {
                contains_case_insensitive(actual, value)
            }
        }
        ActiveTextCondition::Regex(regex) => regex.is_match(actual),
    }
}

fn contains_case_insensitive(actual: &str, lowercase_value: &str) -> bool {
    if lowercase_value.is_empty() {
        return true;
    }
    if actual.is_ascii() && lowercase_value.is_ascii() {
        return actual
            .as_bytes()
            .windows(lowercase_value.len())
            .any(|window| window.eq_ignore_ascii_case(lowercase_value.as_bytes()));
    }
    actual.to_lowercase().contains(lowercase_value)
}

#[cfg(test)]
mod tests;
