//! Pausing Windows background services while a game runs.
//!
//! Stopping services needs administrator rights, so this is split in two:
//!
//! - **One-time setup (elevated, after a UAC prompt):** XBGG copies itself to
//!   `%ProgramFiles%\XBGG\xbgg-helper.exe` (writable by administrators only)
//!   and registers two Task Scheduler tasks that run that copy with fixed
//!   arguments: `--services pause` and `--services resume`.
//! - **Every boost (no prompt):** XBGG starts those tasks.
//!
//! The elevated helper takes no input from the user side: it only stops or
//! starts the services in [`SERVICES`], and only restarts services it stopped
//! itself (recorded in `%ProgramData%\XBGG\services.json`).

use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use windows_sys::Win32::System::Services::{
    CloseServiceHandle, ControlService, OpenSCManagerW, OpenServiceW, QueryServiceStatus, StartServiceW,
    SC_MANAGER_CONNECT, SERVICE_CONTROL_STOP, SERVICE_QUERY_STATUS, SERVICE_RUNNING, SERVICE_START,
    SERVICE_STATUS, SERVICE_STOP, SERVICE_STOPPED,
};

/// The only services XBGG ever touches: background work that competes with
/// games for disk, CPU or network and is safe to pause for a while.
pub const SERVICES: &[(&str, &str)] = &[
    ("WSearch", "Windows Search indexing"),
    ("SysMain", "SysMain (prefetching)"),
    ("DiagTrack", "Telemetry"),
    ("DoSvc", "Delivery Optimization"),
];

pub const TASK_PAUSE: &str = r"\XBGG\Pause services";
pub const TASK_RESUME: &str = r"\XBGG\Resume services";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn is_allowed(name: &str) -> bool {
    SERVICES.iter().any(|(s, _)| s.eq_ignore_ascii_case(name))
}

pub fn helper_exe() -> PathBuf {
    let pf = std::env::var_os("ProgramFiles").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));
    pf.join("XBGG").join("xbgg-helper.exe")
}

/// Next to the helper in Program Files: only administrators can write there,
/// so nobody can plant a link that redirects the elevated helper's writes.
pub fn state_path() -> PathBuf {
    helper_exe().with_file_name("services-state.json")
}

/// A service the helper paused, with the start type to put back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Paused {
    pub name: String,
    pub start_type: u32,
}

/// Services the helper stopped and will start again.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServiceState {
    pub paused: Vec<Paused>,
    pub at: u64,
}

impl ServiceState {
    pub fn names(&self) -> Vec<String> {
        self.paused.iter().map(|p| p.name.clone()).collect()
    }
}

pub fn read_state() -> ServiceState {
    fs::read_to_string(state_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn schtasks(args: &[&str]) -> Result<std::process::Output> {
    let exe = std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    Command::new(exe.join("System32").join("schtasks.exe"))
        .args(args)
        .stdin(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .context("running schtasks")
}

// ------------------------------------------------------- user side

/// Whether the one-time setup has been done.
pub fn helper_installed() -> bool {
    helper_exe().is_file()
        && [TASK_PAUSE, TASK_RESUME]
            .iter()
            .all(|t| schtasks(&["/query", "/tn", t]).is_ok_and(|o| o.status.success()))
}

/// Starts the pause (or resume) task. Returns once the task has been started.
pub fn request(pause: bool) -> Result<()> {
    let task = if pause { TASK_PAUSE } else { TASK_RESUME };
    let out = schtasks(&["/run", "/tn", task])?;
    if !out.status.success() {
        bail!("could not start the {} task: {}", task, String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

// ------------------------------------------------------- elevated side

struct Handle(windows_sys::Win32::System::Services::SC_HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { CloseServiceHandle(self.0) };
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

fn open(name: &str, access: u32) -> Result<(Handle, Handle)> {
    unsafe {
        let scm = OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_CONNECT);
        if scm.is_null() {
            bail!("cannot open the service manager");
        }
        let scm = Handle(scm);
        let svc = OpenServiceW(scm.0, wide(name).as_ptr(), access);
        if svc.is_null() {
            bail!("cannot open service {name} (administrator rights needed?)");
        }
        Ok((scm, Handle(svc)))
    }
}

pub fn service_running(name: &str) -> bool {
    let Ok((_scm, svc)) = open(name, SERVICE_QUERY_STATUS) else { return false };
    let mut st: SERVICE_STATUS = unsafe { std::mem::zeroed() };
    unsafe { QueryServiceStatus(svc.0, &mut st) != 0 && st.dwCurrentState == SERVICE_RUNNING }
}

fn stop(name: &str) -> Result<()> {
    let (_scm, svc) = open(name, SERVICE_STOP | SERVICE_QUERY_STATUS)?;
    let mut st: SERVICE_STATUS = unsafe { std::mem::zeroed() };
    if unsafe { ControlService(svc.0, SERVICE_CONTROL_STOP, &mut st) } == 0 {
        bail!("{name} refused to stop: {}", std::io::Error::last_os_error());
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if unsafe { QueryServiceStatus(svc.0, &mut st) } != 0 && st.dwCurrentState == SERVICE_STOPPED {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    bail!("{name} did not stop in time")
}

fn start(name: &str) -> Result<()> {
    let (_scm, svc) = open(name, SERVICE_START | SERVICE_QUERY_STATUS)?;
    if unsafe { StartServiceW(svc.0, 0, std::ptr::null()) } == 0 {
        let e = std::io::Error::last_os_error();
        // ERROR_SERVICE_ALREADY_RUNNING
        if e.raw_os_error() != Some(1056) {
            bail!("{name} did not start: {e}");
        }
    }
    Ok(())
}

fn write_state(state: &ServiceState) -> Result<()> {
    let path = state_path();
    // Never create the folder here: it must come from the elevated setup
    // (Program Files ACL). Without it, refuse instead of writing elsewhere.
    if !path.parent().is_some_and(|d| d.is_dir()) {
        bail!("helper folder missing; run the admin setup again");
    }
    fs::write(&path, serde_json::to_vec_pretty(state)?).context("writing service state (administrator rights needed)")?;
    Ok(())
}

const SERVICE_DISABLED: u32 = 4;
const SERVICE_NO_CHANGE: u32 = 0xFFFF_FFFF;

fn start_type(name: &str) -> Result<u32> {
    use windows_sys::Win32::System::Services::{QueryServiceConfigW, QUERY_SERVICE_CONFIGW, SERVICE_QUERY_CONFIG};
    let (_scm, svc) = open(name, SERVICE_QUERY_CONFIG)?;
    let mut needed = 0u32;
    unsafe { QueryServiceConfigW(svc.0, std::ptr::null_mut(), 0, &mut needed) };
    let mut buf = vec![0u64; (needed as usize).div_ceil(8).max(8)];
    let cfg = buf.as_mut_ptr() as *mut QUERY_SERVICE_CONFIGW;
    if unsafe { QueryServiceConfigW(svc.0, cfg, (buf.len() * 8) as u32, &mut needed) } == 0 {
        bail!("cannot read the configuration of {name}");
    }
    Ok(unsafe { (*cfg).dwStartType })
}

fn set_start_type(name: &str, start_type: u32) -> Result<()> {
    use windows_sys::Win32::System::Services::{ChangeServiceConfigW, SERVICE_CHANGE_CONFIG};
    let (_scm, svc) = open(name, SERVICE_CHANGE_CONFIG)?;
    let null = std::ptr::null();
    let ok = unsafe {
        ChangeServiceConfigW(
            svc.0,
            SERVICE_NO_CHANGE,
            start_type,
            SERVICE_NO_CHANGE,
            null,
            null,
            std::ptr::null_mut(),
            null,
            null,
            null,
            null,
        )
    };
    if ok == 0 {
        bail!("cannot change the start type of {name}: {}", std::io::Error::last_os_error());
    }
    Ok(())
}

/// `--services pause` (elevated): stops the allowed services that are
/// running. Each is disabled while paused (some, like Windows Search, would
/// otherwise restart on their own); its start type is recorded first.
pub fn pause_now() -> Result<Vec<String>> {
    // Keep services from an earlier pause that was never resumed.
    let mut state = read_state();
    state.paused.retain(|p| is_allowed(&p.name));
    state.at = crate::now_unix();
    let mut errors = Vec::new();
    for (name, _) in SERVICES {
        if !service_running(name) || state.paused.iter().any(|p| p.name == *name) {
            continue;
        }
        let original = match start_type(name) {
            Ok(t) => t,
            Err(e) => {
                errors.push(format!("{e:#}"));
                continue;
            }
        };
        // Record first: if anything fails from here on, resume restores it.
        state.paused.push(Paused { name: name.to_string(), start_type: original });
        write_state(&state)?;
        if let Err(e) = set_start_type(name, SERVICE_DISABLED).and_then(|_| stop(name)) {
            errors.push(format!("{e:#}"));
        }
    }
    write_state(&state)?;
    if !errors.is_empty() {
        bail!("{}", errors.join("\n"));
    }
    Ok(state.names())
}

/// `--services resume` (elevated): restores each paused service's start type
/// and starts it again.
pub fn resume_now() -> Result<Vec<String>> {
    let state = read_state();
    let mut started = Vec::new();
    let mut errors = Vec::new();
    for p in state.paused.iter().filter(|p| is_allowed(&p.name)) {
        // Never leave a service disabled, even if starting it fails.
        if let Err(e) = set_start_type(&p.name, p.start_type) {
            errors.push(format!("{e:#}"));
        }
        if p.start_type == SERVICE_DISABLED {
            continue;
        }
        match start(&p.name) {
            Ok(()) => started.push(p.name.clone()),
            Err(e) => errors.push(format!("{e:#}")),
        }
    }
    let _ = fs::remove_file(state_path());
    if !errors.is_empty() {
        bail!("{}", errors.join("\n"));
    }
    Ok(started)
}

/// Version of the installed helper copy (from its file version info).
pub fn helper_matches(current: &str) -> bool {
    // The helper is a copy of an xbgg.exe; compare file contents' version by
    // asking it, which needs no elevation.
    let Ok(out) = Command::new(helper_exe()).arg("--version").stdin(Stdio::null()).creation_flags(CREATE_NO_WINDOW).output() else {
        return false;
    };
    String::from_utf8_lossy(&out.stdout).trim().ends_with(current)
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn task_xml(user: &str, exe: &Path, action: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>XBGG: {action} Windows Search indexing, SysMain, Telemetry and Delivery Optimization while a game launched from XBGG runs. Fixed list; no other input.</Description>
  </RegistrationInfo>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>Queue</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <ExecutionTimeLimit>PT2M</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{exe}</Command>
      <Arguments>--services {action}</Arguments>
    </Exec>
  </Actions>
</Task>
"#,
        user = xml_escape(user),
        exe = xml_escape(&exe.display().to_string()),
        action = action,
    )
}

/// `--install-service-helper` (elevated): copies `src_exe` to Program Files
/// and registers the two tasks for the signed-in user.
pub fn install(src_exe: &Path) -> Result<()> {
    let dst = helper_exe();
    fs::create_dir_all(dst.parent().unwrap()).context("creating the helper folder (administrator rights needed)")?;
    fs::copy(src_exe, &dst).with_context(|| format!("copying the helper to {}", dst.display()))?;
    let user = match (std::env::var("USERDOMAIN"), std::env::var("USERNAME")) {
        (Ok(d), Ok(u)) => format!("{d}\\{u}"),
        (_, Ok(u)) => u,
        _ => bail!("cannot determine the current user"),
    };
    for (task, action) in [(TASK_PAUSE, "pause"), (TASK_RESUME, "resume")] {
        let xml = task_xml(&user, &dst, action);
        // schtasks wants UTF-16 for an XML that says so.
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend(xml.encode_utf16().flat_map(|u| u.to_le_bytes()));
        let file = std::env::temp_dir().join(format!("xbgg-task-{action}-{}.xml", std::process::id()));
        fs::write(&file, bytes)?;
        let out = schtasks(&["/create", "/tn", task, "/xml", &file.display().to_string(), "/f"]);
        let _ = fs::remove_file(&file);
        let out = out?;
        if !out.status.success() {
            bail!("registering {task}: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
    }
    Ok(())
}

/// `--uninstall-service-helper` (elevated): resumes anything still paused,
/// removes the tasks and the helper copy.
pub fn uninstall() -> Result<()> {
    resume_now()?;
    for task in [TASK_PAUSE, TASK_RESUME] {
        let _ = schtasks(&["/delete", "/tn", task, "/f"]);
    }
    let _ = schtasks(&["/delete", "/tn", r"\XBGG", "/f"]);
    if let Some(dir) = helper_exe().parent() {
        let _ = fs::remove_dir_all(dir);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_known_services_are_allowed() {
        assert!(is_allowed("wsearch") && is_allowed("SysMain"));
        assert!(!is_allowed("WinDefend") && !is_allowed("") && !is_allowed("WSearch "));
    }

    #[test]
    fn task_xml_is_fixed_and_escaped() {
        let xml = task_xml(r"PC\a&b", Path::new(r"C:\Program Files\XBGG\xbgg-helper.exe"), "pause");
        assert!(xml.contains(r"<UserId>PC\a&amp;b</UserId>"));
        assert!(xml.contains("<Arguments>--services pause</Arguments>"));
        assert!(xml.contains("<RunLevel>HighestAvailable</RunLevel>"));
        assert!(xml.contains("<LogonType>InteractiveToken</LogonType>"));
    }

    #[test]
    fn querying_works_without_admin() {
        // Reading service status needs no elevation; WSearch exists on Windows.
        let _ = service_running("WSearch");
        assert!(!service_running("ThisServiceDoesNotExist"));
    }
}
