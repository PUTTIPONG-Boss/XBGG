//! Candidates for the "Apps to freeze" screen.

use std::collections::{BTreeMap, HashSet};

use windows_sys::core::BOOL;
use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, IsWindowVisible};

use super::config::{is_never_freeze, normalize, BoostConfig, SUGGESTED};
use super::sys::{self, Process, ACCESS_QUERY};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppInfo {
    /// Lower-case executable name, e.g. `zen.exe`.
    pub name: String,
    pub processes: usize,
    /// Working set of all its processes, in bytes.
    pub memory: u64,
    pub selected: bool,
}

impl AppInfo {
    pub fn running(&self) -> bool {
        self.processes > 0
    }
}

/// Apps worth offering: everything on the freeze list, the suggestions, and
/// any app in this session that currently has a visible window.
pub fn candidates(cfg: &BoostConfig) -> Vec<AppInfo> {
    let my_session = sys::current_session();
    let me = std::process::id();
    let windowed = pids_with_windows();

    let mut running: BTreeMap<String, (usize, u64, bool)> = BTreeMap::new();
    for p in sys::list_processes() {
        if p.pid == me || sys::session_of(p.pid) != my_session {
            continue;
        }
        let name = normalize(&p.name);
        if is_never_freeze(&name) {
            continue;
        }
        let mem = Process::open(p.pid, ACCESS_QUERY).and_then(|h| h.working_set()).unwrap_or(0);
        let e = running.entry(name).or_default();
        e.0 += 1;
        e.1 += mem;
        e.2 |= windowed.contains(&p.pid);
    }

    let mut names: Vec<String> = cfg.freeze_list.to_vec();
    names.extend(SUGGESTED.iter().map(|s| s.to_string()));
    names.extend(running.iter().filter(|(_, v)| v.2).map(|(k, _)| k.clone()));
    names.sort();
    names.dedup();

    let mut apps: Vec<AppInfo> = names
        .into_iter()
        .filter(|n| !is_never_freeze(n))
        .map(|name| {
            let (processes, memory, _) = running.get(&name).copied().unwrap_or_default();
            let selected = cfg.freeze_list.contains(&name);
            AppInfo { name, processes, memory, selected }
        })
        .collect();
    // Running apps first, biggest first; then the rest alphabetically.
    apps.sort_by(|a, b| b.running().cmp(&a.running()).then(b.memory.cmp(&a.memory)).then(a.name.cmp(&b.name)));
    apps
}

fn pids_with_windows() -> HashSet<u32> {
    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let set = unsafe { &mut *(lparam as *mut HashSet<u32>) };
        if unsafe { IsWindowVisible(hwnd) } != 0 {
            let mut pid = 0u32;
            unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
            set.insert(pid);
        }
        1
    }
    let mut set = HashSet::new();
    unsafe { EnumWindows(Some(collect), &mut set as *mut _ as LPARAM) };
    set
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_configured_and_suggested_apps_without_protected_ones() {
        let mut cfg = BoostConfig::default();
        cfg.freeze_list.push("explorer.exe".into());
        let apps = candidates(&cfg);
        assert!(apps.iter().any(|a| a.name == "chrome.exe" && a.selected));
        assert!(apps.iter().any(|a| a.name == "discord.exe" && !a.selected));
        assert!(!apps.iter().any(|a| a.name == "explorer.exe" || a.name == "steam.exe"));
        // Running apps come first.
        let first_stopped = apps.iter().position(|a| !a.running()).unwrap_or(apps.len());
        assert!(apps[first_stopped..].iter().all(|a| !a.running()));
    }
}
