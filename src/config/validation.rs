use std::collections::HashSet;

use regex::RegexBuilder;

use super::model::{Device, EditableConfig, MatchOperator, SendAction, WindowMatcher};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub path: String,
    pub message: String,
}

impl EditableConfig {
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

pub(super) fn format_errors(errors: &[ValidationError]) -> String {
    errors
        .iter()
        .map(|error| format!("{}: {}", error.path, error.message))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests;
