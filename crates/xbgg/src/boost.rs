//! Runs a boost session alongside a game.
//!
//! A worker thread applies the boost, finds the game's process (anything
//! running from the game's folder), sleeps on it until it exits, then undoes
//! everything. A separate guard process (`xbgg --boost-guard <pid>`) restores
//! the journal if XBGG itself dies mid-session.

use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use xbgg_core::boost::journal;
use xbgg_core::boost::session::{is_game_folder, restore_leftover, ApplyReport, DirScanner, RestoreReport, Session};
use xbgg_core::boost::sys::{Process, ACCESS_WAIT};
use xbgg_core::boost::BoostConfig;

use crate::platform;

/// Looking for the game's process.
const SCAN_EVERY: Duration = Duration::from_secs(2);
/// Game folder empty for this many scans in a row = game closed.
const EMPTY_SCANS_TO_END: u32 = 2;
/// If the game is never recognised (e.g. a shortcut), end the boost once the
/// player has been back in XBGG this long...
const BACK_IN_XBGG: Duration = Duration::from_secs(10);
/// ...but not during the first moments after launching.
const MIN_SESSION: Duration = Duration::from_secs(30);
const TICK: Duration = Duration::from_millis(500);
/// Keep looking for new game processes this long after the first one appears.
const STARTUP_WATCH: Duration = Duration::from_secs(60);
/// Full rescans during the startup window.
const STARTUP_RESCAN: Duration = Duration::from_secs(10);
/// Priority re-checks of known game processes during the startup window.
const PRIORITY_CHECK: Duration = Duration::from_secs(2);

const STOP_NONE: u8 = 0;
const STOP_MENU: u8 = 1;
const STOP_SHUTDOWN: u8 = 2;

pub enum Event {
    Started(ApplyReport),
    GameFound,
    Ended(Summary),
}

pub struct Summary {
    pub restore: RestoreReport,
    pub duration: Duration,
    pub reason: &'static str,
}

#[derive(Default)]
pub struct Controller {
    running: Option<(Arc<AtomicU8>, JoinHandle<()>)>,
    guard: Option<Child>,
}

impl Controller {
    pub fn is_active(&mut self) -> bool {
        if self.running.as_ref().is_some_and(|(_, h)| h.is_finished()) {
            self.running = None;
        }
        self.running.is_some()
    }

    /// Starts boosting for a game that runs from `game_dir` and/or as process
    /// `game_pid` (when XBGG started it directly).
    pub fn start(
        &mut self,
        cfg: BoostConfig,
        game_dir: Option<PathBuf>,
        game_pid: Option<u32>,
        emit: impl Fn(Event) + Send + 'static,
    ) {
        self.stop_blocking(STOP_SHUTDOWN);
        self.ensure_guard();
        let stop = Arc::new(AtomicU8::new(STOP_NONE));
        let flag = stop.clone();
        let handle = thread::Builder::new()
            .name("xbgg-boost".into())
            .spawn(move || run(cfg, game_dir, game_pid, flag, emit))
            .expect("failed to spawn boost thread");
        self.running = Some((stop, handle));
    }

    /// Ends the session from the menu; the `Ended` event follows.
    pub fn stop(&mut self) {
        if let Some((stop, _)) = &self.running {
            stop.store(STOP_MENU, Ordering::SeqCst);
        }
    }

    /// Ends the session and waits until everything is restored.
    pub fn shutdown(&mut self) {
        self.stop_blocking(STOP_SHUTDOWN);
    }

    fn stop_blocking(&mut self, reason: u8) {
        if let Some((stop, handle)) = self.running.take() {
            stop.store(reason, Ordering::SeqCst);
            let _ = handle.join();
        }
    }

    /// Starts the guard process if it isn't running. It restores a boost and
    /// the desktop if XBGG dies.
    pub fn ensure_guard(&mut self) {
        if let Some(g) = &mut self.guard {
            if matches!(g.try_wait(), Ok(None)) {
                return;
            }
        }
        let Ok(exe) = std::env::current_exe() else { return };
        match Command::new(exe).arg("--boost-guard").arg(std::process::id().to_string()).spawn() {
            Ok(child) => self.guard = Some(child),
            Err(e) => eprintln!("xbgg: could not start boost guard: {e}"),
        }
    }
}

fn run(cfg: BoostConfig, game_dir: Option<PathBuf>, game_pid: Option<u32>, stop: Arc<AtomicU8>, emit: impl Fn(Event)) {
    let Some(journal_path) = journal::default_path() else { return };
    let started = Instant::now();
    let (mut session, report) = Session::start(&cfg, game_dir.as_deref(), journal_path);
    emit(Event::Started(report));

    // Only scan folders that belong to the game; a shared folder such as
    // System32 would match unrelated processes.
    let scan_dir = game_dir.as_deref().filter(|d| is_game_folder(d));
    let reason = watch(&mut session, &cfg, scan_dir, game_pid, &stop, &emit);
    let restore = session.end();
    emit(Event::Ended(Summary { restore, duration: started.elapsed(), reason }));
}

fn watch(
    session: &mut Session,
    cfg: &BoostConfig,
    game_dir: Option<&std::path::Path>,
    game_pid: Option<u32>,
    stop: &AtomicU8,
    emit: &impl Fn(Event),
) -> &'static str {
    let started = Instant::now();
    let mut last_scan: Option<Instant> = None;
    let mut boosted = std::collections::HashSet::new();
    let mut scanner = game_dir.map(DirScanner::new);
    let mut seen_at = Instant::now();
    let mut seen = false;
    let mut empty_scans = 0;
    let mut in_xbgg_since: Option<Instant> = None;

    // A process XBGG started itself: sleep on it first.
    if let Some(h) = game_pid.and_then(|pid| Process::open(pid, ACCESS_WAIT)) {
        seen = true;
        seen_at = Instant::now();
        if let Some(pid) = game_pid {
            boosted.insert(pid);
            session.boost_game(cfg, pid);
        }
        emit(Event::GameFound);
        while !h.wait(Some(TICK.as_millis() as u32)) {
            match stop.load(Ordering::SeqCst) {
                STOP_MENU => return "ended from the menu",
                STOP_SHUTDOWN => return "XBGG closed",
                _ => {}
            }
        }
        if game_dir.is_none() {
            return "the game closed";
        }
        // It may have been a launcher: keep watching the game's folder.
    }

    loop {
        match stop.load(Ordering::SeqCst) {
            STOP_MENU => return "ended from the menu",
            STOP_SHUTDOWN => return "XBGG closed",
            _ => {}
        }

        if let Some(scanner) = scanner.as_mut() {
            // Full scans (one process snapshot, ~10 ms): every 2 s until the
            // game shows up, then every 10 s while it starts up.
            let mut every = if seen && empty_scans == 0 { STARTUP_RESCAN } else { SCAN_EVERY };
            if seen && seen_at.elapsed() < STARTUP_WATCH {
                // Cheap checks in between: many games reset their own
                // priority while initialising (put it back), and if every
                // known game process is gone, look again right away.
                let mut any_alive = false;
                for &pid in &boosted {
                    session.boost_game(cfg, pid);
                    any_alive |= Process::open(pid, ACCESS_WAIT).is_some_and(|h| !h.wait(Some(0)));
                }
                if !any_alive {
                    every = SCAN_EVERY;
                }
            }
            if last_scan.is_none_or(|t| t.elapsed() >= every) {
                last_scan = Some(Instant::now());
                let pids = scanner.scan();
                // Games often start several processes, or restart themselves
                // while loading: boost every one we see.
                for &pid in &pids {
                    if boosted.insert(pid) {
                        session.boost_game(cfg, pid);
                    }
                }
                if let Some(&pid) = pids.first() {
                    if !seen {
                        seen = true;
                        seen_at = Instant::now();
                        emit(Event::GameFound);
                    }
                    empty_scans = 0;
                    // While the game is starting up, keep watching it;
                    // afterwards sleep on it until it exits (no polling).
                    if seen_at.elapsed() < STARTUP_WATCH {
                        thread::sleep(PRIORITY_CHECK);
                        continue;
                    }
                    if let Some(h) = Process::open(pid, ACCESS_WAIT) {
                        while !h.wait(Some(TICK.as_millis() as u32)) {
                            if stop.load(Ordering::SeqCst) != STOP_NONE {
                                break;
                            }
                        }
                        last_scan = None; // re-check right away
                    }
                    continue;
                } else if seen {
                    empty_scans += 1;
                    if empty_scans >= EMPTY_SCANS_TO_END {
                        return "the game closed";
                    }
                }
            }
        }

        // Game not recognised: end once the player is back in XBGG for a while.
        if !seen && started.elapsed() >= MIN_SESSION {
            if platform::process_is_foreground() {
                let since = *in_xbgg_since.get_or_insert_with(Instant::now);
                if since.elapsed() >= BACK_IN_XBGG {
                    return "you returned to XBGG";
                }
            } else {
                in_xbgg_since = None;
            }
        }
        thread::sleep(TICK);
    }
}

/// `xbgg --boost-guard <pid>`: waits for XBGG to exit and restores anything
/// it left frozen. Uses no CPU while waiting.
pub fn guard(owner: u32) {
    if let Some(h) = Process::open(owner, ACCESS_WAIT) {
        h.wait(None);
    }
    if let Some(path) = journal::default_path() {
        if journal::load(&path).is_some_and(|j| j.owner_pid == owner) {
            restore_leftover(&path);
        }
    }
    // The desktop, if that XBGG had hidden it.
    xbgg_core::shell::restore_leftover(Some(owner));
}

/// Restores a session interrupted by a crash or power loss.
pub fn recover() -> Option<RestoreReport> {
    restore_leftover(&journal::default_path()?)
}

/// "45 s", "12 min", "1 h 5 min".
pub fn duration(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..=59 => format!("{s} s"),
        60..=3599 => format!("{} min", s / 60),
        _ => format!("{} h {} min", s / 3600, s % 3600 / 60),
    }
}

pub fn gib(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / (1u64 << 30) as f64)
}

pub fn mib(bytes: u64) -> String {
    if bytes >= 1 << 30 {
        gib(bytes)
    } else {
        format!("{} MB", bytes >> 20)
    }
}
