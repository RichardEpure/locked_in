pub(super) struct InstanceGuard;

pub(super) fn claim_single_instance() -> anyhow::Result<Option<InstanceGuard>> {
    anyhow::bail!("single-instance handling is unsupported on this platform")
}

pub(super) fn set_launch_at_login(_enabled: bool) -> anyhow::Result<()> {
    anyhow::bail!("launch-at-login registration is unsupported on this platform")
}

pub(super) fn launch_at_login_enabled() -> anyhow::Result<bool> {
    anyhow::bail!("launch-at-login inspection is unsupported on this platform")
}

pub(super) fn default_data_root() -> anyhow::Result<std::path::PathBuf> {
    anyhow::bail!("default application locations are unsupported on this platform")
}

pub(super) fn edit_file(_path: &std::path::Path) -> anyhow::Result<()> {
    anyhow::bail!("desktop file editing is unsupported on this platform")
}

pub(super) fn open_directory(_path: &std::path::Path) -> anyhow::Result<()> {
    anyhow::bail!("desktop directory opening is unsupported on this platform")
}

pub(super) mod foreground {
    use std::{marker::PhantomData, rc::Rc};

    pub(in crate::platform) struct Monitor(PhantomData<Rc<()>>);

    pub(in crate::platform) fn start() -> anyhow::Result<Monitor> {
        anyhow::bail!("foreground monitoring is unsupported on this platform")
    }
}
