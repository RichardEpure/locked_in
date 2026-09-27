use regex::Regex;

mod compiler;
pub(super) use compiler::validate_action;

use super::{Device, EditableConfig, EventKind, ValidationError};
use crate::{event::Event, focused_window::FocusedWindow};

#[derive(Debug, Clone)]
pub struct CompiledConfig {
    automations: Box<[CompiledAutomation]>,
}

#[derive(Debug)]
pub struct CompiledDispatch<'a> {
    automation_name: &'a str,
    case_name: &'a str,
    report: &'a [u8],
    destinations: &'a [Device],
}

#[derive(Debug, Clone)]
struct CompiledAutomation {
    name: String,
    event: EventKind,
    cases: Box<[CompiledCase]>,
    otherwise_actions: Box<[CompiledAction]>,
}

#[derive(Debug, Clone)]
struct CompiledCase {
    name: String,
    applications: Box<[CompiledWindowMatcher]>,
    exceptions: Box<[CompiledWindowMatcher]>,
    actions: Box<[CompiledAction]>,
}

#[derive(Debug, Clone)]
struct CompiledWindowMatcher {
    title: Option<CompiledTextCondition>,
    class: Option<CompiledTextCondition>,
    exe: Option<CompiledTextCondition>,
}

#[derive(Debug, Clone)]
enum CompiledTextCondition {
    Equals { value: String, case_sensitive: bool },
    Contains { value: String, case_sensitive: bool },
    Regex(Regex),
}

#[derive(Debug, Clone)]
struct CompiledAction {
    report: Box<[u8]>,
    destinations: Box<[Device]>,
}

impl CompiledConfig {
    pub fn compile(editable: &EditableConfig) -> Result<Self, Vec<ValidationError>> {
        compiler::compile(editable)
    }

    pub fn evaluate_event<'a>(&'a self, event: &Event) -> Vec<CompiledDispatch<'a>> {
        let mut dispatches = Vec::new();
        for automation in self
            .automations
            .iter()
            .filter(|automation| automation.event == event.kind())
        {
            let selected = automation.cases.iter().find(|case| case.matches(event));
            let (case_name, actions) = if let Some(case) = selected {
                (case.name.as_str(), case.actions.as_ref())
            } else {
                ("Otherwise", automation.otherwise_actions.as_ref())
            };

            dispatches.extend(actions.iter().map(|action| CompiledDispatch {
                automation_name: &automation.name,
                case_name,
                report: &action.report,
                destinations: &action.destinations,
            }));
        }
        dispatches
    }
}

impl<'a> CompiledDispatch<'a> {
    pub fn automation_name(&self) -> &'a str {
        self.automation_name
    }

    pub fn case_name(&self) -> &'a str {
        self.case_name
    }

    pub fn report(&self) -> &'a [u8] {
        self.report
    }

    pub fn destinations(&self) -> &'a [Device] {
        self.destinations
    }
}

impl CompiledCase {
    fn matches(&self, event: &Event) -> bool {
        match event {
            Event::FocusedWindowChanged { window, .. } => {
                self.applications
                    .iter()
                    .any(|matcher| matcher.matches(window))
                    && !self
                        .exceptions
                        .iter()
                        .any(|matcher| matcher.matches(window))
            }
        }
    }
}

impl CompiledWindowMatcher {
    fn matches(&self, window: &FocusedWindow) -> bool {
        matches_condition(self.title.as_ref(), window.title.as_deref())
            && matches_condition(self.class.as_ref(), window.class.as_deref())
            && matches_condition(
                self.exe.as_ref(),
                window.exe.as_ref().and_then(|value| value.to_str()),
            )
    }
}

fn matches_condition(condition: Option<&CompiledTextCondition>, actual: Option<&str>) -> bool {
    let Some(condition) = condition else {
        return true;
    };
    let Some(actual) = actual else {
        return false;
    };

    match condition {
        CompiledTextCondition::Equals {
            value,
            case_sensitive,
        } => {
            if *case_sensitive {
                actual == value
            } else {
                actual.eq_ignore_ascii_case(value)
            }
        }
        CompiledTextCondition::Contains {
            value,
            case_sensitive,
        } => {
            if *case_sensitive {
                actual.contains(value)
            } else {
                contains_case_insensitive(actual, value)
            }
        }
        CompiledTextCondition::Regex(regex) => regex.is_match(actual),
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
