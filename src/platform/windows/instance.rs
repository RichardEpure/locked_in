use anyhow::Result;
use windows::{
    Win32::{
        Foundation::{ERROR_ALREADY_EXISTS, GetLastError},
        System::Threading::CreateMutexW,
        UI::WindowsAndMessaging::{FindWindowW, SW_RESTORE, SetForegroundWindow, ShowWindow},
    },
    core::{PCWSTR, w},
};

use super::HandleGuard;

pub(in crate::platform) struct InstanceGuard {
    _guard: HandleGuard,
}

pub(in crate::platform) fn claim_single_instance() -> Result<Option<InstanceGuard>> {
    let handle = unsafe { CreateMutexW(None, false, w!("Local\\LockedIn.Desktop.Instance"))? };
    let guard = HandleGuard(handle);
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        match unsafe { FindWindowW(PCWSTR::null(), w!("Locked In")) } {
            Ok(window) if !window.is_invalid() => unsafe {
                let _ = ShowWindow(window, SW_RESTORE);
                if !SetForegroundWindow(window).as_bool() {
                    anyhow::bail!("existing Locked In window could not be focused");
                }
            },
            Ok(_) => anyhow::bail!("existing Locked In instance has no discoverable window"),
            Err(error) => return Err(error.into()),
        }
        return Ok(None);
    }
    Ok(Some(InstanceGuard { _guard: guard }))
}
