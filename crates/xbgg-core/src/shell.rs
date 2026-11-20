//! "Shell mode": start with Windows and hide the desktop while XBGG is open.
//!
//! Instead of replacing the Windows shell in the registry (a mistake there
//! means a black screen at sign-in), XBGG asks Explorer to exit cleanly — the
//! same as "Exit Explorer" on the taskbar context menu — and starts it again
//! when XBGG closes. A crash is covered by the guard process and by
//! [`restore_leftover`] at the next start.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use windows_sys::core::BOOL;
use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{
    OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowW, GetClassNameW, GetWindowThreadProcessId, IsHungAppWindow, IsWindowVisible,
    PostMessageW,
};

/// Message the taskbar handles as "Exit Explorer".
const WM_EXIT_EXPLORER: u32 = 0x5B4;
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
pub const RUN_VALUE: &str = "XBGG";

fn taskbar() -> HWND {
    let class: Vec<u16> = "Shell_TrayWnd".encode_utf16().chain([0]).collect();
    unsafe { FindWindowW(class.as_ptr(), std::ptr::null()) }
}

/// True while the Windows taskbar/desktop (Explorer as the shell) is running.
pub fn desktop_running() -> bool {
    !taskbar().is_null()
}

fn shell_pid() -> Option<u32> {
    let tray = taskbar();
    if tray.is_null() {
        return None;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(tray, &mut pid) };
    (pid != 0).then_some(pid)
}

/// File Explorer windows shown by process `pid`.
pub fn folder_windows(pid: u32) -> usize {
    unsafe extern "system" fn count(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let (pid, n) = unsafe { &mut *(lparam as *mut (u32, usize)) };
        let mut owner = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, &mut owner) };
        if owner == *pid && unsafe { IsWindowVisible(hwnd) } != 0 {
            let mut class = [0u16; 32];
            let len = unsafe { GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32) } as usize;
            if String::from_utf16_lossy(&class[..len]) == "CabinetWClass" {
                *n += 1;
            }
        }
        1
    }
    let mut state = (pid, 0usize);
    unsafe { EnumWindows(Some(count), &mut state as *mut _ as LPARAM) };
    state.1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hide {
    /// The desktop is closed (Explorer exited completely).
    Hidden,
    /// Not attempted: File Explorer windows are open in the shell process.
    /// Explorer would keep running for them and could not be restored
    /// cleanly as the desktop later.
    FolderWindowsOpen(usize),
    /// Explorer could not be closed at all; the desktop is still there.
    Failed,
}

/// Asks Explorer to exit cleanly (like "Exit Explorer") and waits until the
/// process has really ended.
pub fn hide_desktop(timeout: Duration) -> Hide {
    let Some(pid) = shell_pid() else { return Hide::Hidden };
    let open = folder_windows(pid);
    if open > 0 {
        return Hide::FolderWindowsOpen(open);
    }
    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_TERMINATE, 0, pid) };
    unsafe { PostMessageW(taskbar(), WM_EXIT_EXPLORER, 0, 0) };
    if process.is_null() {
        // Can't watch it; fall back to the taskbar disappearing.
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline && desktop_running() {
            std::thread::sleep(Duration::from_millis(100));
        }
        return if desktop_running() { Hide::Failed } else { Hide::Hidden };
    }
    let mut exited = unsafe { WaitForSingleObject(process, timeout.as_millis() as u32) } == WAIT_OBJECT_0;
    if !exited {
        // The taskbar is gone but the process lingers (another program holds
        // a reference to the shell). A lingering shell process would stop a
        // fresh Explorer from becoming the desktop later, so end it now —
        // like "End task" — and restore a clean one when XBGG closes.
        unsafe { TerminateProcess(process, 1) };
        exited = unsafe { WaitForSingleObject(process, 5000) } == WAIT_OBJECT_0;
    }
    unsafe { CloseHandle(process) };
    if exited { Hide::Hidden } else { Hide::Failed }
}

fn explorer_exe() -> PathBuf {
    let windir = std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    windir.join("explorer.exe")
}

/// Starts the desktop. XBGG may itself run inside a job object (some
/// launchers do that) that ends every child when it closes; the desktop must
/// outlive XBGG, so it breaks away from the job when allowed.
fn spawn_explorer() -> Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    let mut cmd = std::process::Command::new(explorer_exe());
    match cmd.creation_flags(CREATE_BREAKAWAY_FROM_JOB).spawn() {
        Ok(_) => Ok(()),
        // The job forbids breaking away: start it normally.
        Err(_) => {
            std::process::Command::new(explorer_exe()).spawn().context("starting explorer.exe")?;
            Ok(())
        }
    }
}

fn responsive() -> bool {
    let tray = taskbar();
    !tray.is_null() && unsafe { IsHungAppWindow(tray) } == 0
}

fn wait_responsive(until: Instant) -> bool {
    while Instant::now() < until {
        if responsive() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    responsive()
}

/// Ends every explorer.exe in this session (last resort when the desktop
/// is stuck).
fn end_all_explorers() {
    use crate::boost::sys::{current_session, list_processes, session_of};
    let me = current_session();
    for p in list_processes() {
        if p.name.eq_ignore_ascii_case("explorer.exe") && session_of(p.pid) == me {
            unsafe {
                let h = OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, 0, p.pid);
                if !h.is_null() {
                    TerminateProcess(h, 1);
                    WaitForSingleObject(h, 5000);
                    CloseHandle(h);
                }
            }
        }
    }
}

/// Brings back a working desktop: starts Explorer if needed and waits for a
/// responsive taskbar. If it doesn't come up (or hangs), every Explorer is
/// ended and a fresh one started.
pub fn show_desktop(timeout: Duration) -> Result<bool> {
    let start = Instant::now();
    if !desktop_running() {
        spawn_explorer()?;
    }
    if wait_responsive(start + timeout / 2) {
        return Ok(true);
    }
    end_all_explorers();
    std::thread::sleep(Duration::from_millis(500));
    if !desktop_running() {
        spawn_explorer()?;
    }
    Ok(wait_responsive(start + timeout))
}

// ------------------------------------------------------- crash-safe state

/// Written while the desktop is hidden, so it can be restored after a crash.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShellState {
    pub desktop_hidden: bool,
    pub owner_pid: u32,
}

pub fn state_path() -> Option<PathBuf> {
    crate::store::default_path().map(|p| p.with_file_name("shell-state.json"))
}

pub fn load_state(path: &Path) -> Option<ShellState> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

pub fn save_state(path: &Path, state: &ShellState) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(state)?)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Brings the desktop back if a previous XBGG hid it and didn't restore it.
/// `owner`: only restore state written by this process (the guard's case).
pub fn restore_leftover(owner: Option<u32>) -> bool {
    let Some(path) = state_path() else { return false };
    let Some(state) = load_state(&path) else { return false };
    if owner.is_some_and(|o| o != state.owner_pid) {
        return false;
    }
    if state.desktop_hidden {
        let _ = show_desktop(Duration::from_secs(20));
    }
    let _ = fs::remove_file(&path);
    state.desktop_hidden
}

// ------------------------------------------------------ start with Windows

/// The command line registered to start XBGG at sign-in.
pub fn autostart_command(exe: &Path) -> String {
    format!("\"{}\" --autostart", exe.display())
}

/// Current autostart command under `value` (`None` if not registered).
pub fn autostart(value: &str) -> Option<String> {
    use winreg::enums::HKEY_CURRENT_USER;
    winreg::RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(RUN_KEY)
        .and_then(|k| k.get_value::<String, _>(value))
        .ok()
}

/// Registers (`Some(exe)`) or removes (`None`) start-at-sign-in for the
/// current user only.
pub fn set_autostart(value: &str, exe: Option<&Path>) -> Result<()> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE};
    let (key, _) = winreg::RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey(RUN_KEY)
        .context("opening the Run key")?;
    match exe {
        Some(exe) => key.set_value(value, &autostart_command(exe)).context("registering autostart")?,
        None => {
            let key = winreg::RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE)?;
            match key.delete_value(value) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e).context("removing autostart"),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autostart_round_trip_under_a_test_name() {
        let name = format!("XBGG-test-{}", std::process::id());
        let exe = Path::new(r"D:\Apps\XBGG\xbgg.exe");
        assert_eq!(autostart(&name), None);
        set_autostart(&name, Some(exe)).unwrap();
        assert_eq!(autostart(&name).as_deref(), Some(r#""D:\Apps\XBGG\xbgg.exe" --autostart"#));
        set_autostart(&name, None).unwrap();
        assert_eq!(autostart(&name), None);
        set_autostart(&name, None).unwrap(); // removing twice is fine
    }

    #[test]
    fn state_round_trip() {
        let path = std::env::temp_dir().join(format!("xbgg-shell-{}.json", std::process::id()));
        let s = ShellState { desktop_hidden: true, owner_pid: 7 };
        save_state(&path, &s).unwrap();
        assert_eq!(load_state(&path), Some(s));
        let _ = fs::remove_file(&path);
    }
}
