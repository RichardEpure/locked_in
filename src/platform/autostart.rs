use anyhow::Result;

use super::native;

/// Applies and confirms launch-at-login synchronously.
///
/// Coordinator reads are allowed during reconciliation; same-thread mutation reentry is
/// rejected. Do not wait for another thread to mutate the same coordinator: its durable
/// operations remain serialized while this interface is called.
pub(crate) trait LaunchAtLogin: Send + Sync {
    fn reconcile(&self, desired: bool) -> LaunchAtLoginOutcome;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LaunchAtLoginState {
    Confirmed(bool),
    Unconfirmed,
}

impl LaunchAtLoginState {
    pub(crate) fn confirmed(self) -> Option<bool> {
        match self {
            Self::Confirmed(confirmed) => Some(confirmed),
            Self::Unconfirmed => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LaunchAtLoginOutcome {
    pub(crate) state: LaunchAtLoginState,
    pub(crate) warning: Option<String>,
}

impl LaunchAtLoginOutcome {
    pub(crate) fn confirmed(confirmed: bool) -> Self {
        Self {
            state: LaunchAtLoginState::Confirmed(confirmed),
            warning: None,
        }
    }

    pub(crate) fn warning(confirmed: bool, warning: impl Into<String>) -> Self {
        Self {
            state: LaunchAtLoginState::Confirmed(confirmed),
            warning: Some(warning.into()),
        }
    }

    pub(crate) fn unconfirmed(warning: impl Into<String>) -> Self {
        Self {
            state: LaunchAtLoginState::Unconfirmed,
            warning: Some(warning.into()),
        }
    }
}

pub(crate) struct SystemLaunchAtLogin;

impl LaunchAtLogin for SystemLaunchAtLogin {
    fn reconcile(&self, desired: bool) -> LaunchAtLoginOutcome {
        reconcile(
            desired,
            native::set_launch_at_login,
            native::launch_at_login_enabled,
        )
    }
}

fn reconcile(
    desired: bool,
    apply: impl FnOnce(bool) -> Result<()>,
    inspect: impl FnOnce() -> Result<bool>,
) -> LaunchAtLoginOutcome {
    match apply(desired) {
        Ok(()) => LaunchAtLoginOutcome::confirmed(desired),
        Err(apply_error) => match inspect() {
            Ok(confirmed) => LaunchAtLoginOutcome::warning(
                confirmed,
                format!("Launch-at-login registration failed: {apply_error:#}"),
            ),
            Err(inspect_error) => LaunchAtLoginOutcome::unconfirmed(format!(
                "Launch-at-login registration failed: {apply_error:#}; the applied state could not be confirmed: {inspect_error:#}"
            )),
        },
    }
}

#[cfg(test)]
mod tests;
