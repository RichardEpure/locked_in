pub(crate) mod autostart;
pub(crate) mod desktop;
pub(crate) mod foreground;
pub(crate) mod instance;
pub(crate) mod locations;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as native;

#[cfg(not(windows))]
mod unsupported;
#[cfg(not(windows))]
use unsupported as native;
