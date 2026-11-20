//! Applying and undoing a boost.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::config::{normalize, BoostConfig};
use super::journal::{self, Journal, PriorityChange, ProcRef};
use super::sys::{self, Process, ACCESS_FREEZE, ACCESS_PRIORITY, ACCESS_QUERY, OVERLAY_BEST_PERFORMANCE};

/// `HIGH_PRIORITY_CLASS`
const PRIORITY_HIGH: u32 = 0x80;

/// What starting a boost did.
#[derive(Debug, Clone, Default)]
pub struct ApplyReport {
    pub frozen_processes: usize,
    /// Distinct app names that were frozen.
    pub frozen_apps: Vec<String>,
    /// Working-set memory released by the frozen apps, in bytes.
    pub trimmed_bytes: u64,
    /// Available physical memory before and shortly after, in bytes.
    pub ram_before: u64,
    pub ram_after: u64,
    pub power_changed: bool,
    pub power_already_best: bool,
    /// Processes we were not allowed to touch (e.g. elevated).
    pub skipped: Vec<String>,
}

/// What ending a boost did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RestoreReport {
    pub resumed: usize,
    /// Frozen processes that had exited (or were replaced) meanwhile.
    pub gone: usize,
    pub power_restored: bool,
    pub priority_restored: usize,
}

pub struct Session {
    journal_path: PathBuf,
    journal: Journal,
}

impl Session {
    /// Applies the boost. `protect_dir` is the game's folder: nothing running
    /// from there is ever frozen.
    pub fn start(cfg: &BoostConfig, protect_dir: Option<&Path>, journal_path: PathBuf) -> (Session, ApplyReport) {
        let mut report = ApplyReport { ram_before: sys::available_ram(), ..Default::default() };
        let mut session = Session {
            journal_path,
            journal: Journal {
                owner_pid: std::process::id(),
                started_at: crate::now_unix(),
                ..Default::default()
            },
        };

        if cfg.power_mode {
            match sys::power_overlay() {
                Some(OVERLAY_BEST_PERFORMANCE) => report.power_already_best = true,
                Some(current) => {
                    session.journal.power_overlay_before = Some(format!("{current:032x}"));
                    session.persist();
                    report.power_changed = sys::set_power_overlay(OVERLAY_BEST_PERFORMANCE);
                    if !report.power_changed {
                        session.journal.power_overlay_before = None;
                        session.persist();
                    }
                }
                None => {}
            }
        }

        if cfg.freeze_apps {
            session.freeze(cfg, protect_dir, &mut report);
            // Released pages become "available" as Windows moves them to the
            // standby list; give it a moment before measuring.
            std::thread::sleep(Duration::from_millis(1500));
        }
        report.ram_after = sys::available_ram();
        (session, report)
    }

    fn freeze(&mut self, cfg: &BoostConfig, protect_dir: Option<&Path>, report: &mut ApplyReport) {
        let me = std::process::id();
        let my_session = sys::current_session();
        let mut targets: Vec<(Process, ProcRef)> = Vec::new();
        let mut apps = BTreeSet::new();

        for p in sys::list_processes() {
            if p.pid == me || p.pid == 0 || !cfg.should_freeze(&p.name) || sys::session_of(p.pid) != my_session {
                continue;
            }
            let Some(handle) = Process::open(p.pid, ACCESS_FREEZE) else {
                report.skipped.push(p.name.clone());
                continue;
            };
            if let (Some(dir), Some(path)) = (protect_dir, handle.image_path()) {
                if path_in(&path, dir) {
                    continue;
                }
            }
            let Some(created) = handle.created() else { continue };
            apps.insert(normalize(&p.name));
            targets.push((handle, ProcRef { pid: p.pid, created, name: p.name }));
        }

        // Record first, act second: a crash after this point is recoverable.
        self.journal.frozen = targets.iter().map(|(_, r)| r.clone()).collect();
        self.persist();

        for (handle, r) in &targets {
            if !handle.suspend() {
                report.skipped.push(r.name.clone());
                continue;
            }
            report.frozen_processes += 1;
            let before = handle.working_set().unwrap_or(0);
            handle.trim();
            let after = handle.working_set().unwrap_or(before);
            report.trimmed_bytes += before.saturating_sub(after);
        }
        report.frozen_apps = apps.into_iter().collect();
        report.skipped.sort();
        report.skipped.dedup();
    }

    /// Raises the game's priority (if enabled), or raises it again if the
    /// game lowered it itself — many games reset their priority while they
    /// initialise. Returns whether it changed anything.
    pub fn boost_game(&mut self, cfg: &BoostConfig, pid: u32) -> bool {
        if !cfg.game_priority {
            return false;
        }
        let Some(h) = Process::open(pid, ACCESS_PRIORITY) else { return false };
        let (Some(created), Some(before)) = (h.created(), h.priority()) else { return false };
        if before == PRIORITY_HIGH {
            return false;
        }
        let known = self.journal.priority.iter().any(|c| c.process.pid == pid && c.process.created == created);
        if known {
            // Already recorded with its original priority: just re-apply.
            return h.set_priority(PRIORITY_HIGH);
        }
        let name = h
            .image_path()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default();
        self.journal.priority.push(PriorityChange { process: ProcRef { pid, created, name }, class_before: before });
        self.persist();
        h.set_priority(PRIORITY_HIGH)
    }

    pub fn frozen_count(&self) -> usize {
        self.journal.frozen.len()
    }

    /// Undoes everything and removes the journal.
    pub fn end(self) -> RestoreReport {
        let report = undo(&self.journal);
        journal::remove(&self.journal_path);
        report
    }

    fn persist(&self) {
        if let Err(e) = journal::save(&self.journal_path, &self.journal) {
            eprintln!("xbgg: could not write boost journal: {e:#}");
        }
    }
}

/// Restores a session left behind by a crash. Returns `None` if there was none.
pub fn restore_leftover(path: &Path) -> Option<RestoreReport> {
    let j = journal::load(path)?;
    let report = undo(&j);
    journal::remove(path);
    Some(report)
}

fn undo(j: &Journal) -> RestoreReport {
    let mut report = RestoreReport::default();
    for r in &j.frozen {
        match open_same(r, ACCESS_FREEZE) {
            Some(h) if h.resume() => report.resumed += 1,
            _ => report.gone += 1,
        }
    }
    for c in &j.priority {
        if let Some(h) = open_same(&c.process, ACCESS_PRIORITY) {
            if h.set_priority(c.class_before) {
                report.priority_restored += 1;
            }
        }
    }
    if let Some(before) = j.power_overlay_before.as_deref().and_then(|s| u128::from_str_radix(s, 16).ok()) {
        // Only undo our own change: if the user picked another mode
        // meanwhile, leave their choice alone.
        if sys::power_overlay() == Some(OVERLAY_BEST_PERFORMANCE) {
            report.power_restored = sys::set_power_overlay(before);
        }
    }
    report
}

/// Opens the process only if it is still the one we recorded.
fn open_same(r: &ProcRef, access: windows_sys::Win32::System::Threading::PROCESS_ACCESS_RIGHTS) -> Option<Process> {
    let h = Process::open(r.pid, access)?;
    (h.created() == Some(r.created)).then_some(h)
}

/// Finds the game's processes repeatedly and cheaply: each PID's executable
/// path is looked up only once, so a rescan costs one process snapshot.
pub struct DirScanner {
    dir: PathBuf,
    session: Option<u32>,
    /// PID -> whether it runs from `dir`.
    known: std::collections::HashMap<u32, bool>,
}

impl DirScanner {
    pub fn new(dir: &Path) -> Self {
        DirScanner { dir: dir.to_path_buf(), session: sys::current_session(), known: Default::default() }
    }

    /// PIDs currently running from the game's folder.
    pub fn scan(&mut self) -> Vec<u32> {
        let live = sys::list_processes();
        let alive: std::collections::HashSet<u32> = live.iter().map(|p| p.pid).collect();
        self.known.retain(|pid, _| alive.contains(pid));
        let mut found = Vec::new();
        for p in live {
            let in_dir = *self.known.entry(p.pid).or_insert_with(|| {
                p.pid != 0
                    && sys::session_of(p.pid) == self.session
                    && Process::open(p.pid, ACCESS_QUERY)
                        .and_then(|h| h.image_path())
                        .is_some_and(|path| path_in(&path, &self.dir))
            });
            if in_dir {
                found.push(p.pid);
            }
        }
        found
    }
}

/// PIDs of processes whose executable lives under `dir` (the running game).
pub fn processes_in(dir: &Path) -> Vec<u32> {
    let my_session = sys::current_session();
    sys::list_processes()
        .into_iter()
        .filter(|p| p.pid != 0 && sys::session_of(p.pid) == my_session)
        .filter(|p| {
            Process::open(p.pid, ACCESS_QUERY)
                .and_then(|h| h.image_path())
                .is_some_and(|path| path_in(&path, dir))
        })
        .map(|p| p.pid)
        .collect()
}

/// Whether every process running from `dir` can be assumed to be the game.
/// False for shared folders (Windows, Program Files, a drive root, ...).
pub fn is_game_folder(dir: &Path) -> bool {
    use std::path::Component;
    let depth = dir.components().filter(|c| matches!(c, Component::Normal(_))).count();
    if depth < 2 {
        return false; // e.g. "D:\" or "D:\Games"
    }
    let shared = ["WINDIR", "ProgramFiles", "ProgramFiles(x86)", "ProgramData", "USERPROFILE", "APPDATA", "LOCALAPPDATA"];
    let is_shared_root = shared
        .iter()
        .filter_map(|v| std::env::var_os(v).map(PathBuf::from))
        .any(|root| {
            let windows = std::env::var_os("WINDIR").map(PathBuf::from);
            // Anything under Windows is shared; elsewhere only the root itself.
            if windows.as_deref() == Some(root.as_path()) {
                path_in(dir, &root)
            } else {
                path_in(&root, dir)
            }
        });
    !is_shared_root
}

/// Case-insensitive "is `path` inside `dir`" for Windows paths.
pub fn path_in(path: &Path, dir: &Path) -> bool {
    let norm = |p: &Path| {
        let mut s = p.to_string_lossy().replace('/', "\\").to_lowercase();
        while s.ends_with('\\') {
            s.pop();
        }
        s
    };
    let (p, d) = (norm(path), norm(dir));
    !d.is_empty() && (p == d || p.starts_with(&(d + "\\")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use std::time::Instant;

    #[test]
    fn shared_folders_are_not_game_folders() {
        let win = PathBuf::from(std::env::var("WINDIR").unwrap());
        assert!(!is_game_folder(&win.join("System32")));
        assert!(!is_game_folder(Path::new(r"D:\")));
        assert!(!is_game_folder(Path::new(r"D:\Games")));
        if let Ok(pf) = std::env::var("ProgramFiles") {
            assert!(!is_game_folder(Path::new(&pf)));
            assert!(is_game_folder(&Path::new(&pf).join("Some Game")));
        }
        assert!(is_game_folder(Path::new(r"D:\Steam\steamapps\common\Deadlock")));
        assert!(is_game_folder(Path::new(r"D:\Games\Foo")));
    }

    #[test]
    fn path_matching() {
        let dir = Path::new(r"D:\Steam\steamapps\common\Deadlock");
        assert!(path_in(Path::new(r"d:\steam\steamapps\common\deadlock\game\bin\win64\deadlock.exe"), dir));
        assert!(path_in(Path::new(r"D:/Steam/steamapps/common/Deadlock/x.exe"), Path::new(r"D:\Steam\steamapps\common\Deadlock\")));
        assert!(!path_in(Path::new(r"D:\Steam\steamapps\common\DeadlockTools\x.exe"), dir));
        assert!(!path_in(Path::new(r"C:\x.exe"), Path::new("")));
    }

    /// These tests freeze every `ping.exe` in the session, so they must not
    /// overlap (a doubly-suspended process would stay frozen).
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn serial() -> std::sync::MutexGuard<'static, ()> {
        SERIAL.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Spawns `ping -t` (prints a line every second) and reports the lines.
    fn ticker() -> (std::process::Child, mpsc::Receiver<Instant>) {
        let mut child = Command::new("ping")
            .args(["-t", "127.0.0.1"])
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn ping");
        let out = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for _ in BufReader::new(out).lines() {
                if tx.send(Instant::now()).is_err() {
                    break;
                }
            }
        });
        (child, rx)
    }

    fn lines_within(rx: &mpsc::Receiver<Instant>, d: Duration) -> usize {
        let end = Instant::now() + d;
        let mut n = 0;
        while let Some(left) = end.checked_duration_since(Instant::now()) {
            if rx.recv_timeout(left).is_ok() {
                n += 1;
            }
        }
        n
    }

    #[test]
    fn freezes_and_restores_a_real_process() {
        let _g = serial();
        let (mut child, rx) = ticker();
        assert!(lines_within(&rx, Duration::from_millis(2500)) >= 1, "ping should be printing");

        let journal_path = std::env::temp_dir().join(format!("xbgg-boost-test-{}.json", std::process::id()));
        let cfg = BoostConfig {
            enabled: true,
            freeze_apps: true,
            freeze_list: vec!["ping.exe".into()],
            power_mode: false,
            game_priority: false,
        };
        let (session, report) = Session::start(&cfg, None, journal_path.clone());
        assert!(report.frozen_processes >= 1, "{report:?}");
        assert_eq!(report.frozen_apps, vec!["ping.exe".to_string()]);
        assert!(journal::load(&journal_path).is_some(), "journal written before acting");

        while rx.try_recv().is_ok() {}
        assert_eq!(lines_within(&rx, Duration::from_millis(2500)), 0, "frozen process must not run");

        let restored = session.end();
        assert!(restored.resumed >= 1);
        assert!(journal::load(&journal_path).is_none(), "journal removed after restore");
        assert!(lines_within(&rx, Duration::from_millis(3000)) >= 1, "process runs again");

        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn leftover_journal_from_a_crash_is_restored() {
        let _g = serial();
        let (mut child, rx) = ticker();
        lines_within(&rx, Duration::from_millis(1500));
        let journal_path = std::env::temp_dir().join(format!("xbgg-boost-crash-{}.json", std::process::id()));
        let cfg = BoostConfig {
            freeze_list: vec!["ping.exe".into()],
            power_mode: false,
            ..BoostConfig::default()
        };
        let (session, _) = Session::start(&cfg, None, journal_path.clone());
        // Simulate a crash: the session is lost without `end()`.
        std::mem::forget(session);
        while rx.try_recv().is_ok() {}
        assert_eq!(lines_within(&rx, Duration::from_millis(2000)), 0);

        let r = restore_leftover(&journal_path).expect("journal present");
        assert!(r.resumed >= 1);
        assert!(lines_within(&rx, Duration::from_millis(3000)) >= 1);
        assert!(restore_leftover(&journal_path).is_none());

        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn protected_dir_is_not_frozen() {
        let _g = serial();
        let (mut child, rx) = ticker();
        lines_within(&rx, Duration::from_millis(1500));
        let journal_path = std::env::temp_dir().join(format!("xbgg-boost-protect-{}.json", std::process::id()));
        let cfg = BoostConfig { freeze_list: vec!["ping.exe".into()], power_mode: false, ..BoostConfig::default() };
        let sys32 = std::env::var("WINDIR").map(|w| PathBuf::from(w).join("System32")).unwrap();
        let (session, report) = Session::start(&cfg, Some(&sys32), journal_path);
        assert_eq!(report.frozen_processes, 0, "ping lives in System32, which is protected here");
        session.end();
        let _ = child.kill();
        let _ = child.wait();
    }
}
