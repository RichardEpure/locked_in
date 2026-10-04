use std::path::PathBuf;

use anyhow::Result;

use super::native;

pub(crate) fn default_data_root() -> Result<PathBuf> {
    native::default_data_root()
}
