use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    os::windows::ffi::OsStringExt,
    path::PathBuf,
    sync::{LazyLock, RwLock},
};
use tokio::sync::watch;
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE, HWND},
        System::Threading::{
            OpenProcess, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
            QueryFullProcessImageNameW,
        },
        UI::{
            Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent},
            WindowsAndMessaging::{
                EVENT_SYSTEM_FOREGROUND, GetClassNameW, GetWindowTextLengthW, GetWindowTextW,
                GetWindowThreadProcessId, OBJID_WINDOW, WINEVENT_OUTOFCONTEXT,
            },
        },
    },
    core::PWSTR,
};

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct WindowMetadata {
    pub title: Option<String>,
    pub class: Option<String>,
    pub exe: Option<PathBuf>,
}

impl WindowMetadata {
    pub fn match_any(&self, window: &WindowMetadata) -> bool {
        if let Some(title_self) = &self.title
            && let Some(title_other) = &window.title
            && title_self.contains(title_other)
        {
            return true;
        }

        if let Some(class_self) = &self.class
            && let Some(class_other) = &window.class
            && class_self.contains(class_other)
        {
            return true;
        }

        if let Some(exe_self) = &self.exe
            && let Some(exe_other) = &window.exe
            && exe_self == exe_other
        {
            return true;
        }

        false
    }
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

pub static FOCUSED_WINDOW: LazyLock<RwLock<WindowMetadata>> =
    LazyLock::new(|| RwLock::new(WindowMetadata::default()));

pub static FOCUSED_WINDOW_TX: LazyLock<watch::Sender<WindowMetadata>> = LazyLock::new(|| {
    let (tx, _rx) = watch::channel(WindowMetadata::default());
    tx
});

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
    match FOCUSED_WINDOW.read() {
        Ok(guard) => guard.clone(),
        Err(e) => {
            panic!("get_focused_window: failed to acquire read lock: {}", e);
        }
    }
}

pub fn set_focused_window(window: WindowMetadata) {
    match FOCUSED_WINDOW.write() {
        Ok(mut guard) => {
            *guard = window.clone();
            let _ = FOCUSED_WINDOW_TX.send(window);
        }
        Err(e) => {
            println!("update_focused_window: failed to acquire write lock: {}", e);
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
