use anyhow::Result;
use std::{
    ffi::OsString,
    os::windows::ffi::OsStringExt,
    path::PathBuf,
    sync::{LazyLock, Mutex},
};

pub use crate::focused_window::{FocusedWindow as WindowMetadata, ForegroundObservation};
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
                EVENT_SYSTEM_FOREGROUND, FindWindowW, GetClassNameW, GetForegroundWindow,
                GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, OBJID_WINDOW,
                SW_RESTORE, SetForegroundWindow, ShowWindow, WINEVENT_OUTOFCONTEXT,
            },
        },
    },
    core::{PCWSTR, PWSTR, w},
};

const ALT_TAB_HOST_CLASS: &str = "XamlExplorerHostIslandWindow";

#[derive(Debug, PartialEq, Eq)]
struct ObservationTicket {
    sequence: u64,
    raw_hwnd: isize,
}

#[derive(Default)]
struct ObservationState {
    latest_sequence: u64,
    generation: u64,
    last_raw_hwnd: Option<isize>,
}

impl ObservationState {
    fn begin(&mut self, raw_hwnd: isize) -> Option<ObservationTicket> {
        if self.last_raw_hwnd.replace(raw_hwnd) == Some(raw_hwnd) {
            return None;
        }

        self.latest_sequence = self
            .latest_sequence
            .checked_add(1)
            .expect("foreground observation sequence overflow");
        Some(ObservationTicket {
            sequence: self.latest_sequence,
            raw_hwnd,
        })
    }

    fn complete(
        &mut self,
        ticket: ObservationTicket,
        window: WindowMetadata,
    ) -> Option<ForegroundObservation> {
        if ticket.sequence != self.latest_sequence {
            return None;
        }
        if window.class.as_deref() == Some(ALT_TAB_HOST_CLASS) {
            return None;
        }

        self.generation = self
            .generation
            .checked_add(1)
            .expect("foreground observation generation overflow");
        Some(ForegroundObservation {
            generation: self.generation,
            raw_hwnd: ticket.raw_hwnd,
            window,
        })
    }
}

struct ForegroundPublisher {
    state: Mutex<ObservationState>,
    observations: watch::Sender<ForegroundObservation>,
    metadata: watch::Sender<WindowMetadata>,
}

impl ForegroundPublisher {
    fn new() -> Self {
        let (observations, _) = watch::channel(ForegroundObservation::default());
        let (metadata, _) = watch::channel(WindowMetadata::default());
        Self {
            state: Mutex::new(ObservationState::default()),
            observations,
            metadata,
        }
    }

    fn begin(&self, raw_hwnd: isize) -> Option<ObservationTicket> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .begin(raw_hwnd)
    }

    fn begin_with<T>(
        &self,
        raw_observation: impl FnOnce() -> Option<(isize, T)>,
    ) -> Option<(ObservationTicket, T)> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (raw_hwnd, value) = raw_observation()?;
        Some((state.begin(raw_hwnd)?, value))
    }

    fn complete(
        &self,
        ticket: ObservationTicket,
        window: WindowMetadata,
    ) -> Option<ForegroundObservation> {
        self.complete_before_versioned(ticket, window, || {})
    }

    fn complete_before_versioned(
        &self,
        ticket: ObservationTicket,
        window: WindowMetadata,
        before_versioned: impl FnOnce(),
    ) -> Option<ForegroundObservation> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let observation = state.complete(ticket, window)?;
        self.metadata.send_replace(observation.window.clone());
        before_versioned();
        self.observations.send_replace(observation.clone());
        Some(observation)
    }

    fn subscribe_observations(&self) -> watch::Receiver<ForegroundObservation> {
        self.observations.subscribe()
    }

    fn subscribe_metadata(&self) -> watch::Receiver<WindowMetadata> {
        self.metadata.subscribe()
    }

    fn metadata(&self) -> WindowMetadata {
        self.metadata.borrow().clone()
    }
}

static FOREGROUND_PUBLISHER: LazyLock<ForegroundPublisher> =
    LazyLock::new(ForegroundPublisher::new);

#[expect(dead_code, reason = "the versioned receiver is consumed by L-0012")]
pub fn subscribe_foreground_observations() -> watch::Receiver<ForegroundObservation> {
    FOREGROUND_PUBLISHER.subscribe_observations()
}

pub fn subscribe_focused_window() -> watch::Receiver<WindowMetadata> {
    FOREGROUND_PUBLISHER.subscribe_metadata()
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
    FOREGROUND_PUBLISHER.metadata()
}

fn resolve_window_metadata(hwnd: HWND) -> WindowMetadata {
    WindowMetadata {
        title: hwnd_title(hwnd),
        class: hwnd_class(hwnd),
        exe: hwnd_pid(hwnd).and_then(process_exe),
    }
}

fn observe_foreground_window(hwnd: HWND) {
    if hwnd.is_invalid() {
        return;
    }

    let Some(ticket) = FOREGROUND_PUBLISHER.begin(hwnd.0 as isize) else {
        return;
    };
    FOREGROUND_PUBLISHER.complete(ticket, resolve_window_metadata(hwnd));
}

fn reconcile_foreground_window() {
    let Some((ticket, hwnd)) = FOREGROUND_PUBLISHER.begin_with(|| {
        let hwnd = unsafe { GetForegroundWindow() };
        (!hwnd.is_invalid()).then_some((hwnd.0 as isize, hwnd))
    }) else {
        return;
    };
    FOREGROUND_PUBLISHER.complete(ticket, resolve_window_metadata(hwnd));
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

    reconcile_foreground_window();
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

    observe_foreground_window(hwnd);
}

#[cfg(test)]
mod tests;
