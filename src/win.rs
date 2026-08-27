use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    os::windows::ffi::OsStringExt,
    path::PathBuf,
    sync::{LazyLock, Mutex},
};
use tokio::sync::watch;
use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND},
        System::Threading::{
            CreateMutexW, OpenProcess, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
            QueryFullProcessImageNameW,
        },
        UI::{
            Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent},
            WindowsAndMessaging::{
                EVENT_SYSTEM_FOREGROUND, FindWindowW, GetClassNameW, GetWindowTextLengthW,
                GetWindowTextW, GetWindowThreadProcessId, OBJID_WINDOW, SW_RESTORE,
                SetForegroundWindow, ShowWindow, WINEVENT_OUTOFCONTEXT,
            },
        },
    },
    core::{PCWSTR, PWSTR, w},
};

pub static FOCUSED_WINDOW: LazyLock<Mutex<WindowMetadata>> =
    LazyLock::new(|| Mutex::new(WindowMetadata::default()));

pub static FOCUSED_WINDOW_TX: LazyLock<watch::Sender<WindowMetadata>> = LazyLock::new(|| {
    let (tx, _rx) = watch::channel(WindowMetadata::default());
    tx
});

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct WindowMetadata {
    pub title: Option<String>,
    pub class: Option<String>,
    pub exe: Option<PathBuf>,
}

pub struct WinHook {
    hook: HWINEVENTHOOK,
}
impl Drop for WinHook {
    fn drop(&mut self) {
        unsafe {
            let _ = UnhookWinEvent(self.hook);
        }
    }
}

struct HandleGuard(HANDLE);
impl Drop for HandleGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

#[inline]
fn hwnd_title(hwnd: HWND) -> Option<String> {
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        let mut buf = vec![0u16; (len + 1) as usize];
        let n = GetWindowTextW(hwnd, &mut buf);
        if n > 0 {
            buf.truncate(n as usize);
            Some(OsString::from_wide(&buf).to_string_lossy().into_owned())
        } else {
            None
        }
    }
}

#[inline]
fn hwnd_class(hwnd: HWND) -> Option<String> {
    unsafe {
        let mut buf = vec![0u16; 256];
        let n = GetClassNameW(hwnd, &mut buf);
        if n > 0 {
            buf.truncate(n as usize);
            Some(OsString::from_wide(&buf).to_string_lossy().into_owned())
        } else {
            None
        }
    }
}

#[inline]
fn hwnd_pid(hwnd: HWND) -> Option<u32> {
    let mut pid = 0u32;
    unsafe {
        let _tid = GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    if pid == 0 { None } else { Some(pid) }
}

#[inline]
fn process_exe(pid: u32) -> Option<PathBuf> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        if handle.is_invalid() {
            return None;
        }
        let _guard = HandleGuard(handle);

        let mut buf = vec![0u16; 1024];
        let mut size = buf.len() as u32;
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_FORMAT(0),
            PWSTR(buf.as_mut_ptr()),
            &mut size,
        )
        .ok()?;

        buf.truncate(size as usize);
        Some(OsString::from_wide(&buf).into())
    }
}

pub fn get_focused_window() -> WindowMetadata {
    match FOCUSED_WINDOW.lock() {
        Ok(guard) => guard.clone(),
        Err(e) => {
            panic!("get_focused_window: failed to acquire lock: {}", e);
        }
    }
}

pub fn set_focused_window(window: WindowMetadata) {
    match FOCUSED_WINDOW.lock() {
        Ok(mut guard) => {
            *guard = window.clone();
            let _ = FOCUSED_WINDOW_TX.send(window);
        }
        Err(e) => {
            eprintln!("update_focused_window: failed to acquire lock: {}", e);
        }
    }
}

pub fn start_foreground_hook() -> Result<WinHook> {
    let flags = WINEVENT_OUTOFCONTEXT;

    let hook = unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(win_event_proc),
            0,
            0,
            flags,
        )
    };

    if hook.is_invalid() {
        anyhow::bail!("SetWinEventHook failed");
    }

    Ok(WinHook { hook })
}

pub struct InstanceGuard {
    _guard: HandleGuard,
}

pub fn claim_single_instance() -> Result<Option<InstanceGuard>> {
    let handle = unsafe { CreateMutexW(None, false, w!("Local\\LockedIn.Desktop.Instance"))? };
    let guard = HandleGuard(handle);
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        match unsafe { FindWindowW(PCWSTR::null(), w!("Locked In")) } {
            Ok(window) if !window.is_invalid() => unsafe {
                let _ = ShowWindow(window, SW_RESTORE);
                if !SetForegroundWindow(window).as_bool() {
                    eprintln!("existing Locked In window could not be focused");
                }
            },
            Ok(_) => eprintln!("existing Locked In instance has no discoverable window"),
            Err(error) => eprintln!("existing Locked In window lookup failed: {error}"),
        }
        return Ok(None);
    }
    Ok(Some(InstanceGuard { _guard: guard }))
}

pub fn set_start_with_windows(enabled: bool) -> Result<()> {
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
        let query = std::process::Command::new("reg.exe")
            .args(["query", key, "/v", value_name])
            .status()?;
        if !query.success() {
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

unsafe extern "system" fn win_event_proc(
    _hwineventhook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    _idchild: i32,
    _ideventthread: u32,
    _dwmseventtime: u32,
) {
    if event != EVENT_SYSTEM_FOREGROUND {
        println!("win_event_proc: event != EVENT_SYSTEM_FOREGROUND");
        return;
    }

    if id_object != OBJID_WINDOW.0 {
        println!("win_event_proc: id_object != OBJID_WINDOW");
        return;
    }

    let window = WindowMetadata {
        title: hwnd_title(hwnd),
        class: hwnd_class(hwnd),
        exe: hwnd_pid(hwnd).and_then(process_exe),
    };

    // Ignore alt + tab 'window'
    if let Some(class) = &window.class
        && class == "XamlExplorerHostIslandWindow"
    {
        return;
    }

    set_focused_window(window);
}
