use std::path::Path;

use anyhow::Result;

use super::native;

pub(crate) fn edit_file(path: &Path) -> Result<()> {
    native::edit_file(path)
}

pub(crate) fn open_directory(path: &Path) -> Result<()> {
    native::open_directory(path)
}
