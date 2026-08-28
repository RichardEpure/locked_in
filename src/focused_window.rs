use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FocusedWindow {
    pub title: Option<String>,
    pub class: Option<String>,
    pub exe: Option<PathBuf>,
}
