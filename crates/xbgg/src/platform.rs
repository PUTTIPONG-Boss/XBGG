//! Small Win32 helpers.

use slint::ComponentHandle;

use crate::AppWindow;

/// True when XBGG's window is the foreground window. Controller input is
/// read globally, so this keeps button presses made in a game from reaching
/// the launcher behind it.
#[cfg(windows)]
pub fn is_foreground(ui: &AppWindow) -> bool {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    let handle = ui.window().window_handle();
    let Ok(wh) = handle.window_handle() else { return true };
    let RawWindowHandle::Win32(h) = wh.as_raw() else { return true };
    let fg = unsafe { GetForegroundWindow() };
    fg as isize == h.hwnd.get()
}

#[cfg(not(windows))]
pub fn is_foreground(_ui: &AppWindow) -> bool {
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
