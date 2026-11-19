//! Small Win32 helpers.

/// True when a window owned by this process is in the foreground.
///
/// Controller input is read globally, so this keeps button presses made in a
/// game from reaching the launcher behind it. Thread-safe.
#[cfg(windows)]
pub fn process_is_foreground() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

    let mut pid = 0u32;
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_null() {
            return false;
        }
        GetWindowThreadProcessId(fg, &mut pid);
    }
    pid == std::process::id()
}

#[cfg(not(windows))]
pub fn process_is_foreground() -> bool {
    true
}

/// Lets the process we are about to start (or the launcher that starts it)
/// take the foreground, so a game does not open behind the full-screen shell.
#[cfg(windows)]
pub fn allow_foreground_handoff() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, ASFW_ANY};
    unsafe {
        AllowSetForegroundWindow(ASFW_ANY);
    }
}

#[cfg(not(windows))]
pub fn allow_foreground_handoff() {}

/// Held for the lifetime of the process; while it exists, other XBGG
/// instances defer to this one.
pub struct InstanceGuard(#[allow(dead_code)] isize);

#[cfg(windows)]
impl Drop for InstanceGuard {
    fn drop(&mut self) {
        if self.0 != 0 {
            unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0 as _) };
        }
    }
}

/// Takes the single-instance lock, waiting up to `wait` for a previous
/// instance to exit (used when restarting after an update). Returns `None`
/// if another instance keeps running.
#[cfg(windows)]
pub fn single_instance(wait: std::time::Duration) -> Option<InstanceGuard> {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::System::Threading::CreateMutexW;

    let name: Vec<u16> = "Local\\XBGG.SingleInstance".encode_utf16().chain([0]).collect();
    let deadline = std::time::Instant::now() + wait;
    loop {
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        if handle.is_null() {
            // Could not create the lock at all: don't block the user.
            return Some(InstanceGuard(0));
        }
        if unsafe { GetLastError() } != ERROR_ALREADY_EXISTS {
            return Some(InstanceGuard(handle as isize));
        }
        unsafe { CloseHandle(handle) };
        if std::time::Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

#[cfg(not(windows))]
pub fn single_instance(_wait: std::time::Duration) -> Option<InstanceGuard> {
    Some(InstanceGuard(0))
}

/// Brings the already running XBGG window to the front.
#[cfg(windows)]
pub fn focus_existing_instance() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE,
    };
    let title: Vec<u16> = "XBGG".encode_utf16().chain([0]).collect();
    unsafe {
        let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
        if hwnd.is_null() {
            return;
        }
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        SetForegroundWindow(hwnd);
    }
}

#[cfg(not(windows))]
pub fn focus_existing_instance() {}

/// Release builds use the GUI subsystem; attach to the launching terminal so
/// command-line modes (`--list`, `--version`, ...) can print.
#[cfg(windows)]
pub fn attach_console() {
    use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(windows))]
pub fn attach_console() {}
