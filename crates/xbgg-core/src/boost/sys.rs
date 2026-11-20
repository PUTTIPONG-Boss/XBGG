//! Thin, safe wrappers over the Win32 calls Boost needs.
//!
//! Everything here works with normal user rights: we only touch processes in
//! the user's own session, and only with the access rights each call needs.

use std::path::PathBuf;

use windows_sys::core::GUID;
use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::ProcessStatus::{K32EmptyWorkingSet, K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows_sys::Win32::System::Threading::{
    GetPriorityClass, GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, SetPriorityClass,
    WaitForSingleObject, PROCESS_ACCESS_RIGHTS, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_INFORMATION,
    PROCESS_SET_QUOTA, PROCESS_SUSPEND_RESUME, PROCESS_SYNCHRONIZE,
};

/// A process from a snapshot.
#[derive(Debug, Clone)]
pub struct ProcEntry {
    pub pid: u32,
    pub parent: u32,
    /// Executable file name, e.g. `chrome.exe`.
    pub name: String,
}

pub fn list_processes() -> Vec<ProcEntry> {
    let mut out = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut e) != 0;
        while ok {
            let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            out.push(ProcEntry {
                pid: e.th32ProcessID,
                parent: e.th32ParentProcessID,
                name: String::from_utf16_lossy(&e.szExeFile[..len]),
            });
            ok = Process32NextW(snap, &mut e) != 0;
        }
        CloseHandle(snap);
    }
    out
}

pub fn session_of(pid: u32) -> Option<u32> {
    let mut sid = 0u32;
    (unsafe { ProcessIdToSessionId(pid, &mut sid) } != 0).then_some(sid)
}

pub fn current_session() -> Option<u32> {
    session_of(std::process::id())
}

/// An open process handle, closed on drop.
pub struct Process(HANDLE);

impl Drop for Process {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

// Process handles may be used from any thread.
unsafe impl Send for Process {}

pub const ACCESS_FREEZE: PROCESS_ACCESS_RIGHTS =
    PROCESS_SUSPEND_RESUME | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SET_QUOTA;
pub const ACCESS_QUERY: PROCESS_ACCESS_RIGHTS = PROCESS_QUERY_LIMITED_INFORMATION;
pub const ACCESS_WAIT: PROCESS_ACCESS_RIGHTS = PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION;
pub const ACCESS_PRIORITY: PROCESS_ACCESS_RIGHTS = PROCESS_SET_INFORMATION | PROCESS_QUERY_LIMITED_INFORMATION;

impl Process {
    pub fn open(pid: u32, access: PROCESS_ACCESS_RIGHTS) -> Option<Self> {
        let h = unsafe { OpenProcess(access, 0, pid) };
        (!h.is_null()).then_some(Process(h))
    }

    /// Creation time (FILETIME ticks). Together with the PID this identifies a
    /// process even after its PID is reused.
    pub fn created(&self) -> Option<u64> {
        let z = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let (mut c, mut e, mut k, mut u) = (z, z, z, z);
        if unsafe { GetProcessTimes(self.0, &mut c, &mut e, &mut k, &mut u) } == 0 {
            return None;
        }
        Some(((c.dwHighDateTime as u64) << 32) | c.dwLowDateTime as u64)
    }

    pub fn image_path(&self) -> Option<PathBuf> {
        let mut buf = vec![0u16; 1024];
        let mut len = buf.len() as u32;
        if unsafe { QueryFullProcessImageNameW(self.0, 0, buf.as_mut_ptr(), &mut len) } == 0 {
            return None;
        }
        Some(PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])))
    }

    pub fn working_set(&self) -> Option<u64> {
        let mut c: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
        c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        (unsafe { K32GetProcessMemoryInfo(self.0, &mut c, c.cb) } != 0).then_some(c.WorkingSetSize as u64)
    }

    /// Asks Windows to page out the process' working set. The memory comes
    /// back on demand; for a frozen process that only happens after resume.
    pub fn trim(&self) -> bool {
        unsafe { K32EmptyWorkingSet(self.0) != 0 }
    }

    pub fn suspend(&self) -> bool {
        nt_call("NtSuspendProcess", self.0)
    }

    pub fn resume(&self) -> bool {
        nt_call("NtResumeProcess", self.0)
    }

    pub fn priority(&self) -> Option<u32> {
        let p = unsafe { GetPriorityClass(self.0) };
        (p != 0).then_some(p)
    }

    pub fn set_priority(&self, class: u32) -> bool {
        unsafe { SetPriorityClass(self.0, class) != 0 }
    }

    /// Waits for the process to exit. `None` waits forever.
    pub fn wait(&self, timeout_ms: Option<u32>) -> bool {
        unsafe { WaitForSingleObject(self.0, timeout_ms.unwrap_or(u32::MAX)) == WAIT_OBJECT_0 }
    }
}

/// `NtSuspendProcess` / `NtResumeProcess`: stable ntdll exports used by every
/// process manager; looked up at runtime because no import library has them.
fn nt_call(name: &str, h: HANDLE) -> bool {
    type NtFn = unsafe extern "system" fn(HANDLE) -> i32;
    let Some(f) = proc_address("ntdll.dll", name) else { return false };
    let f: NtFn = unsafe { std::mem::transmute(f) };
    unsafe { f(h) >= 0 }
}

fn proc_address(dll: &str, name: &str) -> Option<unsafe extern "system" fn() -> isize> {
    let wide: Vec<u16> = dll.encode_utf16().chain([0]).collect();
    let cname: Vec<u8> = name.bytes().chain([0]).collect();
    unsafe {
        let m = LoadLibraryW(wide.as_ptr());
        if m.is_null() {
            return None;
        }
        GetProcAddress(m, cname.as_ptr())
    }
}

/// Physical memory currently available to programs, in bytes.
pub fn available_ram() -> u64 {
    let mut s: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    s.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    if unsafe { GlobalMemoryStatusEx(&mut s) } == 0 {
        return 0;
    }
    s.ullAvailPhys
}

// ------------------------------------------------------------- power mode

/// Windows' power mode slider ("overlay" schemes). Values from the Windows SDK.
pub const OVERLAY_BEST_PERFORMANCE: u128 = 0xded574b5_45a0_4f42_8737_46345c09c238;

/// Current power mode overlay, as a GUID number.
pub fn power_overlay() -> Option<u128> {
    type GetFn = unsafe extern "system" fn(*mut GUID) -> u32;
    let f = proc_address("powrprof.dll", "PowerGetEffectiveOverlayScheme")?;
    let f: GetFn = unsafe { std::mem::transmute(f) };
    let mut g = GUID::from_u128(0);
    (unsafe { f(&mut g) } == 0).then(|| guid_to_u128(&g))
}

pub fn set_power_overlay(value: u128) -> bool {
    type SetFn = unsafe extern "system" fn(GUID) -> u32;
    let Some(f) = proc_address("powrprof.dll", "PowerSetActiveOverlayScheme") else { return false };
    let f: SetFn = unsafe { std::mem::transmute(f) };
    unsafe { f(GUID::from_u128(value)) == 0 }
}

fn guid_to_u128(g: &GUID) -> u128 {
    ((g.data1 as u128) << 96)
        | ((g.data2 as u128) << 80)
        | ((g.data3 as u128) << 64)
        | u64::from_be_bytes(g.data4) as u128
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guid_round_trip() {
        let g = GUID::from_u128(OVERLAY_BEST_PERFORMANCE);
        assert_eq!(guid_to_u128(&g), OVERLAY_BEST_PERFORMANCE);
    }

    #[test]
    fn sees_own_process() {
        let me = std::process::id();
        let all = list_processes();
        assert!(all.iter().any(|p| p.pid == me));
        let p = Process::open(me, ACCESS_QUERY).unwrap();
        assert!(p.created().is_some());
        assert!(p.image_path().is_some());
        assert!(p.working_set().unwrap() > 0);
        assert_eq!(session_of(me), current_session());
        assert!(available_ram() > 0);
    }
}
