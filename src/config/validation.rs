use super::{EditableConfig, SendAction};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub path: String,
    pub message: String,
}

impl EditableConfig {
    pub fn validate_action(&self, action: &SendAction) -> Vec<ValidationError> {
        super::compiled::validate_action(self, action)
    }
}

#[cfg(test)]
mod tests;
