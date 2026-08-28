use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FocusedWindow {
    pub title: Option<String>,
    pub class: Option<String>,
    pub exe: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForegroundObservation {
    pub generation: u64,
    pub raw_hwnd: isize,
    pub window: FocusedWindow,
}
