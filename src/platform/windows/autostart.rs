use anyhow::{Result, anyhow};
use windows::{
    Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR},
        System::Registry::{
            HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
        },
    },
    core::w,
};

pub(in crate::platform) fn set_launch_at_login(enabled: bool) -> Result<()> {
    let value_name = "LockedIn";
    let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    let status = if enabled {
        let executable = std::env::current_exe()?;
        std::process::Command::new("reg.exe")
            .args(["add", key, "/v", value_name, "/t", "REG_SZ", "/d"])
            .arg(format!("\"{}\"", executable.display()))
            .args(["/f"])
            .status()?
    } else {
        if !launch_at_login_enabled()? {
            return Ok(());
        }
        std::process::Command::new("reg.exe")
            .args(["delete", key, "/v", value_name, "/f"])
            .status()?
    };
    if !status.success() {
        anyhow::bail!("Windows startup registration failed");
    }
    Ok(())
}

pub(in crate::platform) fn launch_at_login_enabled() -> Result<bool> {
    startup_registry_value_exists(open_startup_registry_key, locked_in_startup_value_exists)
}

struct RegistryKey(HKEY);

impl Drop for RegistryKey {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

fn open_startup_registry_key() -> Result<Option<RegistryKey>> {
    let mut key = HKEY::default();
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Run"),
            None,
            KEY_QUERY_VALUE,
            &mut key,
        )
    };
    match status {
        ERROR_SUCCESS => Ok(Some(RegistryKey(key))),
        ERROR_FILE_NOT_FOUND => Ok(None),
        status => Err(registry_query_error("open the Windows Run key", status)),
    }
}

fn locked_in_startup_value_exists(key: &RegistryKey) -> Result<bool> {
    let mut size = 0;
    let status =
        unsafe { RegQueryValueExW(key.0, w!("LockedIn"), None, None, None, Some(&mut size)) };
    match status {
        ERROR_SUCCESS => Ok(true),
        ERROR_FILE_NOT_FOUND => Ok(false),
        status => Err(registry_query_error(
            "query the LockedIn Windows Run value",
            status,
        )),
    }
}

fn startup_registry_value_exists<K>(
    open_key: impl FnOnce() -> Result<Option<K>>,
    value_exists: impl FnOnce(&K) -> Result<bool>,
) -> Result<bool> {
    match open_key()? {
        Some(key) => value_exists(&key),
        None => Ok(false),
    }
}

fn registry_query_error(operation: &str, status: WIN32_ERROR) -> anyhow::Error {
    anyhow!(
        "failed to {operation}: {}",
        windows::core::Error::from(status)
    )
}

#[cfg(test)]
mod tests;
