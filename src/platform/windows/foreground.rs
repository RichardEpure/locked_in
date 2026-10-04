use std::{
    ffi::OsString, marker::PhantomData, os::windows::ffi::OsStringExt, path::PathBuf, rc::Rc,
    sync::LazyLock,
};

use anyhow::Result;
use windows::{
    Win32::{
        Foundation::HWND,
        System::Threading::{
            OpenProcess, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
            QueryFullProcessImageNameW,
        },
        UI::{
            Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent},
            WindowsAndMessaging::{
                EVENT_SYSTEM_FOREGROUND, GetClassNameW, GetForegroundWindow, GetWindowTextLengthW,
                GetWindowTextW, GetWindowThreadProcessId, OBJID_WINDOW, WINEVENT_OUTOFCONTEXT,
            },
        },
    },
    core::PWSTR,
};

use super::HandleGuard;
use crate::{
    focused_window::FocusedWindow,
    platform::foreground::{Publisher, get, start_monitor},
};

const ALT_TAB_HOST_CLASS: &str = "XamlExplorerHostIslandWindow";

static PUBLISHER: LazyLock<Publisher<isize>> = LazyLock::new(|| Publisher::new(get().clone()));

pub(in crate::platform) struct Monitor {
    hook: HWINEVENTHOOK,
    // The hook must be removed on its registering thread, even if native handle
    // types later gain Send/Sync implementations.
    _registering_thread: PhantomData<Rc<()>>,
}

impl Drop for Monitor {
    fn drop(&mut self) {
        if !unsafe { UnhookWinEvent(self.hook) }.as_bool() {
            let error = windows::core::Error::from_thread();
            crate::app_log::write_error(format!("foreground monitoring cleanup failed: {error}"));
        }
    }
}

pub(in crate::platform) fn start() -> Result<Monitor> {
    start_monitor(
        || {
            let hook = unsafe {
                SetWinEventHook(
                    EVENT_SYSTEM_FOREGROUND,
                    EVENT_SYSTEM_FOREGROUND,
                    None,
                    Some(win_event_proc),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT,
                )
            };
            if hook.is_invalid() {
                anyhow::bail!("SetWinEventHook failed");
            }
            Ok(Monitor {
                hook,
                _registering_thread: PhantomData,
            })
        },
        reconcile_foreground_window,
    )
}

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

fn hwnd_pid(hwnd: HWND) -> Option<u32> {
    let mut pid = 0u32;
    unsafe {
        let _tid = GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    if pid == 0 { None } else { Some(pid) }
}

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

fn resolve_window_metadata(hwnd: HWND) -> Option<FocusedWindow> {
    let window = FocusedWindow {
        title: hwnd_title(hwnd),
        class: hwnd_class(hwnd),
        exe: hwnd_pid(hwnd).and_then(process_exe),
    };
    (window.class.as_deref() != Some(ALT_TAB_HOST_CLASS)).then_some(window)
}

fn observe_foreground_window(hwnd: HWND) {
    if hwnd.is_invalid() {
        return;
    }
    let Some(ticket) = PUBLISHER.begin(hwnd.0 as isize) else {
        return;
    };
    PUBLISHER.complete(ticket, resolve_window_metadata(hwnd));
}

fn reconcile_foreground_window() {
    // Sample native focus while holding ordering admission. An intervening
    // callback cannot be followed by a ticket for an older sampled window.
    let Some((ticket, hwnd)) = PUBLISHER.begin_with(|| {
        let hwnd = unsafe { GetForegroundWindow() };
        (!hwnd.is_invalid()).then_some((hwnd.0 as isize, hwnd))
    }) else {
        return;
    };
    PUBLISHER.complete(ticket, resolve_window_metadata(hwnd));
}

unsafe extern "system" fn win_event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    _id_child: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    if event == EVENT_SYSTEM_FOREGROUND && id_object == OBJID_WINDOW.0 {
        observe_foreground_window(hwnd);
    }
}
