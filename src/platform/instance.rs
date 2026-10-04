use anyhow::Result;

use super::native;

pub(crate) struct InstanceGuard {
    _native: native::InstanceGuard,
}

/// Owns the primary instance, or activates the existing window and returns None.
/// Acquisition and activation failures are reported; a secondary process always exits.
pub(crate) fn claim() -> Result<Option<InstanceGuard>> {
    native::claim_single_instance().map(|owner| owner.map(|owner| InstanceGuard { _native: owner }))
}
