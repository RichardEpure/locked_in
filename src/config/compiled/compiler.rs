use std::collections::{HashMap, HashSet};

use regex::RegexBuilder;

use super::{
    CompiledAction, CompiledAutomation, CompiledCase, CompiledConfig, CompiledTextCondition,
    CompiledWindowMatcher,
};
use crate::config::{
    Device, EditableConfig, MatchOperator, SendAction, TextCondition, ValidationError,
    WindowMatcher,
};

/// Compilation owns semantic diagnostics and the compiled values together. Invalid
/// partial results never escape; disabled drafts are checked but not activated.
pub(super) fn compile(editable: &EditableConfig) -> Result<CompiledConfig, Vec<ValidationError>> {
    let mut errors = Vec::new();
    let mut devices = HashMap::new();
    for (index, device) in editable.devices.iter().enumerate() {
        let path = format!("devices[{index}]");
        if device.id.trim().is_empty() {
            error(&mut errors, &path, "id is required");
        } else if devices.contains_key(device.id.as_str()) {
            error(&mut errors, &path, "id must be unique");
        } else {
            devices.insert(device.id.as_str(), device);
        }
        if device.name.trim().is_empty() {
            error(&mut errors, &path, "name is required");
        }
        if device.report_length == 0 {
            error(
                &mut errors,
                &path,
                "report length must be greater than zero",
            );
        }
    }
    let mut automation_ids = HashSet::new();
    let mut automations = Vec::new();
    for (index, automation) in editable.automations.iter().enumerate() {
        let path = format!("automations[{index}]");
        validate_id(&automation.id, &path, &mut automation_ids, &mut errors);
        if automation.name.trim().is_empty() {
            error(&mut errors, &path, "name is required");
        }
        if automation.enabled
            && automation.cases.is_empty()
            && automation.otherwise_actions.is_empty()
        {
            error(
                &mut errors,
                &path,
                "enabled automation has no cases or otherwise actions",
            );
        }
        let mut case_ids = HashSet::new();
        let mut action_ids = HashSet::new();
        let mut cases = Vec::new();
        for (index, case) in automation.cases.iter().enumerate() {
            let case_path = format!("{path}.cases[{index}]");
            validate_id(&case.id, &case_path, &mut case_ids, &mut errors);
            if automation.enabled && case.applications.is_empty() {
                error(
                    &mut errors,
                    &case_path,
                    "enabled case needs an application matcher",
                );
            }
            if automation.enabled && case.actions.is_empty() {
                error(&mut errors, &case_path, "enabled case needs an action");
            }
            let applications = compile_matchers(
                &case.applications,
                &format!("{case_path}.applications"),
                automation.enabled,
                &mut errors,
            );
            let exceptions = compile_matchers(
                &case.exceptions,
                &format!("{case_path}.exceptions"),
                automation.enabled,
                &mut errors,
            );
            let mut matcher_ids = HashSet::new();
            for matcher in case.applications.iter().chain(&case.exceptions) {
                if matcher.id.trim().is_empty() {
                    error(&mut errors, &case_path, "matcher id is required");
                } else if !matcher_ids.insert(&matcher.id) {
                    error(&mut errors, &case_path, "matcher ids must be unique");
                }
            }
            for action in &case.actions {
                validate_id(&action.id, &case_path, &mut action_ids, &mut errors);
            }
            let actions = compile_actions(
                &case.actions,
                &devices,
                &format!("{case_path}.actions"),
                automation.enabled,
                &mut errors,
            );
            cases.push(CompiledCase {
                name: case.name.clone(),
                applications,
                exceptions,
                actions,
            });
        }
        for action in &automation.otherwise_actions {
            validate_id(&action.id, &path, &mut action_ids, &mut errors);
        }
        let otherwise_actions = compile_actions(
            &automation.otherwise_actions,
            &devices,
            &format!("{path}.otherwise_actions"),
            automation.enabled,
            &mut errors,
        );
        if automation.enabled {
            automations.push(CompiledAutomation {
                name: automation.name.clone(),
                event: automation.event,
                cases: cases.into_boxed_slice(),
                otherwise_actions,
            });
        }
    }
    if errors.is_empty() {
        Ok(CompiledConfig {
            automations: automations.into_boxed_slice(),
        })
    } else {
        Err(errors)
    }
}

fn compile_matchers(
    matchers: &[WindowMatcher],
    path: &str,
    require_complete: bool,
    errors: &mut Vec<ValidationError>,
) -> Box<[CompiledWindowMatcher]> {
    matchers
        .iter()
        .enumerate()
        .map(|(index, matcher)| {
            let path = format!("{path}[{index}]");
            if require_complete
                && matcher.title.is_none()
                && matcher.class.is_none()
                && matcher.exe.is_none()
            {
                error(errors, &path, "at least one field is required");
            }
            let mut condition = |field: &str, value: Option<&TextCondition>| {
                let path = format!("{path}.{field}");
                if require_complete && value.is_some_and(|value| value.value.is_empty()) {
                    error(errors, &path, "value is required");
                }
                match compile_condition(value, &path) {
                    Ok(compiled) => compiled,
                    Err(failure) => {
                        errors.push(failure);
                        None
                    }
                }
            };
            CompiledWindowMatcher {
                title: condition("title", matcher.title.as_ref()),
                class: condition("class", matcher.class.as_ref()),
                exe: condition("exe", matcher.exe.as_ref()),
            }
        })
        .collect()
}

fn compile_actions(
    actions: &[SendAction],
    devices: &HashMap<&str, &Device>,
    path: &str,
    require_complete: bool,
    errors: &mut Vec<ValidationError>,
) -> Box<[CompiledAction]> {
    actions
        .iter()
        .enumerate()
        .map(|(index, action)| {
            let path = format!("{path}[{index}]");
            if require_complete && action.report.is_empty() {
                error(errors, &path, "report is required");
            }
            if require_complete && action.device_ids.is_empty() {
                error(errors, &path, "at least one destination is required");
            }
            let mut seen = HashSet::new();
            let mut destinations = Vec::new();
            for id in &action.device_ids {
                if !seen.insert(id) {
                    error(errors, &path, "destinations must not be duplicated");
                }
                let Some(device) = devices.get(id.as_str()) else {
                    error(errors, &path, &format!("unknown device '{id}'"));
                    continue;
                };
                if action.report.len() > device.report_length as usize {
                    error(
                        errors,
                        &path,
                        &format!(
                            "report exceeds {} byte capacity of {}",
                            device.report_length, device.name
                        ),
                    );
                }
                destinations.push((*device).clone());
            }
            CompiledAction {
                report: action.report.clone().into_boxed_slice(),
                destinations: destinations.into_boxed_slice(),
            }
        })
        .collect()
}

pub(in crate::config) fn validate_action(
    config: &EditableConfig,
    action: &SendAction,
) -> Vec<ValidationError> {
    let devices = config
        .devices
        .iter()
        .map(|device| (device.id.as_str(), device))
        .collect();
    let mut errors = Vec::new();
    compile_actions(
        std::slice::from_ref(action),
        &devices,
        "action",
        true,
        &mut errors,
    );
    errors
}

fn error(errors: &mut Vec<ValidationError>, path: &str, message: &str) {
    errors.push(ValidationError {
        path: path.into(),
        message: message.into(),
    });
}

fn compile_condition(
    condition: Option<&TextCondition>,
    path: &str,
) -> Result<Option<CompiledTextCondition>, ValidationError> {
    let Some(condition) = condition else {
        return Ok(None);
    };
    let compiled = match condition.operator {
        MatchOperator::Equals => CompiledTextCondition::Equals {
            value: condition.value.clone(),
            case_sensitive: condition.case_sensitive,
        },
        MatchOperator::Contains => CompiledTextCondition::Contains {
            value: if condition.case_sensitive {
                condition.value.clone()
            } else {
                condition.value.to_lowercase()
            },
            case_sensitive: condition.case_sensitive,
        },
        MatchOperator::Regex => CompiledTextCondition::Regex(
            RegexBuilder::new(&condition.value)
                .case_insensitive(!condition.case_sensitive)
                .build()
                .map_err(|_| ValidationError {
                    path: path.to_string(),
                    message: "invalid regular expression".into(),
                })?,
        ),
    };
    Ok(Some(compiled))
}

fn validate_id<'a>(
    id: &'a str,
    path: &str,
    seen: &mut HashSet<&'a str>,
    errors: &mut Vec<ValidationError>,
) {
    if id.trim().is_empty() {
        error(errors, path, "id is required");
    } else if !seen.insert(id) {
        error(errors, path, "id must be unique");
    }
}

#[cfg(test)]
mod tests;
