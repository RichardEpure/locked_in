use regex::RegexBuilder;

use super::model::{
    Device, EditableConfig, MatchOperator, SendAction, TextCondition, WindowMatcher,
};
use crate::focused_window::FocusedWindow;

#[derive(Debug)]
pub struct EvaluatedAction<'a> {
    pub automation_name: &'a str,
    pub case_name: &'a str,
    pub action: &'a SendAction,
    pub devices: Vec<&'a Device>,
}

impl EditableConfig {
    pub fn evaluate_window<'a>(&'a self, window: &FocusedWindow) -> Vec<EvaluatedAction<'a>> {
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
}

impl WindowMatcher {
    fn matches(&self, window: &FocusedWindow) -> bool {
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

#[cfg(test)]
mod tests;
