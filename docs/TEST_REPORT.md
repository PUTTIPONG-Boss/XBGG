# XBGG — Test Report

Date: 2026-11-20 · Build: `release` profile (thin LTO, `opt-level = 3`), Rust 1.99.0,
`x86_64-pc-windows-gnu`.

- **v0.5.1**: after the shell-mode test the user's Start menu and search stopped working — explorer.exe was gone. Cause: the test tooling runs inside a job object that ends all child processes; the Explorer that XBGG restored was XBGG's child and ended with it. Fix: Explorer is started with `CREATE_BREAKAWAY_FROM_JOB` (normal start as fallback). Verified: XBGG hid and restored the desktop inside the job, and that Explorer (with Start menu and search hosts) was still running after the job closed.
- **v0.5.0** (shell mode, Windows services): see [v0.5.0](#v050--shell-mode-and-windows-services) below.
- **v0.4.0** (layout switch, square everything): see [v0.4.0](#v040--two-layouts) below.
- **v0.3.0** (terminal UI): see [v0.3.0](#v030--terminal-ui) below.
- **v0.2.0** (Boost mode): see [Boost](#boost) and [v0.2.0 regression](#v020-regression)
  below. Final binary `xbgg.exe` v0.2.0, SHA-256
  `1d56ac83a0fac7c73907f08b9606a392192d52de669adc8d035dbb7030217670`: the full
  Boost end-to-end test passed on it (19/19: 1522 MB → 0 MB, +1373 MB
  available, restored 2.3 s after exit, crash recovery OK), and Microsoft
  Defender found no threats in it or the zip.
- v0.1.1 (`c0498094…a0e5db`) differs from v0.1.0 only by the keyboard-layout
  fix (bug #10).

## v0.5.0 — shell mode and Windows services

### Shell mode — `tools/shell-test.ps1`

Closes and restores the real taskbar several times; open File Explorer windows
are recorded and reopened, `library.json` restored, and the desktop always
brought back.

| Check | Result |
|---|---|
| Taskbar closes when XBGG starts with *hide desktop* on | ✅ |
| The desktop's Explorer process really exits (ended after 4 s if it lingers) | ✅ |
| State recorded before hiding; guard running; XBGG in front | ✅ |
| Exit from the menu → **responsive** taskbar and shell window back, state removed, guard gone | ✅ |
| **Crash** (XBGG killed) → guard brings back a responsive desktop | ✅ |
| Menu toggle on/off closes/restores the desktop live | ✅ |
| *start with windows* adds/removes `HKCU\…\Run\XBGG` = `"…\xbgg.exe" --autostart` | ✅ |
| File Explorer window inside the desktop process → desktop left alone, message shown | ✅ (separate run) |
| Available RAM with the desktop closed | +140 to +330 MB across runs |

Final runs: 18/18 twice in a row.

**Bugs found here and fixed before release** (each one left the test
machine's taskbar broken until the test's failsafe restarted Explorer):

| # | Problem | Fix |
|---|---|---|
| 1 | With a File Explorer window open, Explorer stayed alive after *Exit Explorer*; the Explorer started on restore then hung (taskbar not responding, no desktop window). | Refuse to hide while the desktop process shows File Explorer windows; check the restored taskbar is responsive. |
| 2 | When another program holds a reference to the shell, Explorer lingers without a taskbar. The first fix ended it and expected Windows to restart it (AutoRestartShell) — Windows didn't, and XBGG had already given up on restoring it. | End the lingering process and treat the desktop as hidden, so XBGG restores it on exit; restoring now replaces a missing or hung taskbar with a fresh Explorer. |

### Pausing Windows services

| Check | Result |
|---|---|
| Unit tests: allowlist, fixed task XML, service query without admin | ✅ |
| `--services pause` without admin: refuses, exit code 1, services untouched, **no folder created** | ✅ |
| One-time setup via UAC: helper in `Program Files\XBGG`, both tasks registered; folder writable only by Administrators / SYSTEM / TrustedInstaller | ✅ |
| Non-admin `schtasks /run` of the tasks pauses and resumes the services (no prompt) | ✅ |
| Menu shows *update helper* when the helper is older than XBGG; updating via UAC works | ✅ |
| `boost-test.ps1 -Services` with Deadlock: services **stopped + Disabled** while playing, back to **Running / Automatic** after the game, guard resumes them after a crash | ✅ 26/26 |

**Found and fixed during these tests:**

| # | Problem | Fix |
|---|---|---|
| 3 | **Security**: the services state file was in `%ProgramData%\XBGG`, a folder a normal user can create — a planted link there could redirect the elevated helper's writes. | State now lives next to the helper in `Program Files\XBGG` (admins only); the helper never creates folders itself. |
| 4 | Windows Search restarted itself seconds after being stopped. | Services are disabled while paused; the original start type is recorded first and restored on resume. |

### Regression on v0.5.0

Unit tests 37/37, clippy clean; `bench.ps1` 14.6 MB / 0 MB VRAM; `soak.ps1`
150 cycles peak 16.7 MB, handles 293 → 281; `focus-soak.ps1` threads 13 → 13,
handles 291 → 291; `custom-entry-test.ps1` PASS; `game-launch-test.ps1`
(Deadlock: XBGG 16 ms CPU over 30 s, back in front after exit);
`update-test.ps1` PASS; 20 layout switches bounded. Real update of the published v0.4.0 to v0.5.0 through the menu: PASS (SHA-256 matches the release, no leftovers).

## v0.4.0 — two layouts

The console (carousel) layout is back as an option next to the terminal
layout, with square tiles, labels, keys and checkboxes.

| Check | Result |
|---|---|
| Switch to console via *Menu → layout*; choice saved to `library.json` | ✅ |
| Screens reviewed in console layout: home, carousel move, menu, apps to freeze, exit dialog, add tile | ✅ square everywhere |
| Unknown `layout` value from a newer version → falls back to terminal, no warning, file kept | ✅ unit test |
| `bench.ps1` console / terminal | 25.9 MB / 13.9 MB private, 0 MB VRAM in both |
| `leak-isolate.ps1 -Mode nav` in console, 600 rounds (covers + background art) | flat, 25.7–25.9 MB |
| `leak-isolate.ps1 -Mode layout`, 60 switches | settles at ~17.4 MB in terminal after the first switches (heap keeps freed pages for reuse), no growth afterwards |
| Unit tests | 32/32; clippy clean |
| `soak.ps1 -Cycles 150` | peak 17.1 MB, handles 290 → 280, GDI 17 → 17 |
| `focus-soak.ps1 -Switches 30` | threads 13 → 13, handles 291 → 291 (25/30 foreground hand-overs: the user's Deadlock was running full screen during this run) |
| `custom-entry-test.ps1`, `update-test.ps1` | PASS |
| `game-launch-test.ps1`, `boost-test.ps1` | not re-run: the user was playing Deadlock at the time; Boost code unchanged since v0.3.0 |
| Real update, published binaries: v0.3.0 → Menu → install update → v0.4.0 | PASS (exited 4 s after confirming, SHA-256 matches the release, no leftovers, settings kept) |

## v0.3.0 — terminal UI

The carousel, background art and animations were replaced by a list +
details layout. Measured against v0.2.0 on the same machine:

| Metric (`bench.ps1 -Fullscreen`) | v0.2.0 | v0.3.0 |
|---|---|---|
| Private memory | 25.5 MB | **14.6 MB** |
| CPU for 12 moves | ~1100 ms | **~200 ms** |
| Startup to window | ~350 ms | ~340 ms |
| GPU memory | 0 MB | 0 MB |

Leak hunting on the new UI (`tools/leak-isolate.ps1` repeats one action):

| Action | Result |
|---|---|
| Open menu + app list, 150× | flat (14.8 → 14.6 MB) |
| Rescan library, 400× | flat (14.2 → 14.1 MB) |
| Move up/down (each move loads a cover), 1000× — **first version** | rose to a 16.6 MB plateau: one thread was spawned per cover decode, fragmenting the heap |
| same, after switching to one long-lived cover worker | plateau **14.9 MB**, flat over 2000 cover loads |
| `soak.ps1 -Cycles 300` (all actions) | peak 16.2 MB in the first 50 cycles and never higher afterwards; handles 292 → 287, GDI 17 → 17 |

Regression on v0.3.0: unit tests 30/30, clippy clean, `custom-entry-test`
PASS, `focus-soak` 40/40 (threads 13 → 9, handles 297 → 290),
`game-launch-test` (Deadlock: XBGG 31 ms CPU over 30 s, back in front after
exit), `boost-test` 19/19 (+1378 MB available, restored 2.3 s after exit),
`update-test` PASS.

**Real update, published binaries only**: v0.2.0 downloaded from GitHub,
Menu → *install update* → confirm; it exited 5 s later and restarted as
v0.3.0 (SHA-256 equals the published checksum, no leftover files), showing
the new interface. In the first of these runs the restarted app showed row 3
selected instead of row 1; three identical re-runs (and a start with the
post-update flag set) all selected row 1, so it was most likely real input
reaching the machine during the test, not reproducible.

## Boost

### Unit and integration tests (`cargo test --release`, core)

| Test | What it proves |
|---|---|
| `freezes_and_restores_a_real_process` | A live `ping -t` stops printing while frozen and prints again after restore; journal is written before acting and removed after. |
| `leftover_journal_from_a_crash_is_restored` | Session dropped without `end()` (simulated crash): `restore_leftover` resumes the process from the journal. |
| `protected_dir_is_not_frozen` | Nothing running from the game's folder is ever frozen. |
| `protected_processes_are_never_frozen` | `steam.exe`, `explorer.exe`, `svchost.exe` etc. can't be frozen even if added to the list. |
| `shared_folders_are_not_game_folders` | `System32`, `Program Files`, drive roots are never treated as a game's folder. |
| `lists_configured_and_suggested_apps_without_protected_ones` | The "Apps to freeze" list. |
| journal round-trip, config defaults/back-compat, GUID round-trip, vertical controller input | |

### End to end with a real game — `tools/boost-test.ps1`

Freeze list for the test: `memhog.exe` (a test program holding 1.5 GB and
touching it every second, like a real app) and `wallpaper32.exe` (Wallpaper
Engine, running on the test machine). No other user app was touched. Game:
Deadlock via Steam. High priority enabled for the test.

Final run (all 19 checks):

| Check | Result |
|---|---|
| memhog frozen while the game runs | ✅ |
| memhog working set | ✅ 1522 MB → 0 MB |
| Wallpaper Engine frozen | ✅ |
| Game at High priority (Deadlock resets its own priority ~6 s after start; Boost re-applies it) | ✅ High, still High 30 s later |
| Game itself not frozen | ✅ |
| Journal written, guard process running | ✅ |
| XBGG CPU while boosting the running game (30 s, first minute of play) | ✅ 31 ms XBGG + 0 ms guard |
| Everything resumed after the game closed | ✅ **2.4 s** after exit |
| Journal removed | ✅ |
| Available RAM gained by freezing memhog (measured with nothing else loading) | ✅ **+1328 MB** (5.9 → 7.2 GB) |
| **Crash test**: XBGG force-killed mid-boost → guard restored memhog and Wallpaper Engine, removed the journal, then exited | ✅ within 1 s |

Issues the test found and that were fixed before release:

| Found | Problem | Fix |
|---|---|---|
| Run 2 | Deadlock resets its own priority to Normal ~6 s after it starts, undoing "High priority". | Re-apply during the game's first 60 s (cheap check every 2 s), then stop. |
| Run 3 | Re-scanning every process every 2 s during start-up cost 156 ms CPU / 30 s. | `DirScanner` looks up each PID once; full rescans every 10 s; 31 ms / 30 s. |
| Run 4 | Apps stayed frozen ~20 s after the game closed. | Liveness check of known game processes every 2 s → resumed 2.4 s after exit. |
| Review | A custom `.exe` in a shared folder (e.g. `System32`) would have made unrelated processes count as "the game", so the boost would never end. | Track the PID XBGG starts; only scan folders that belong to the game. |
| Test harness | "All threads suspended" was too strict: a thread in a kernel wait shows *Executive* until the suspension applies when it returns. | Test checks: no thread running and ≥ 90 % suspended. |

### v0.2.0 regression

| Test | Result |
|---|---|
| Unit tests | 31 passed, 0 failed |
| `cargo clippy --all-targets` | no warnings |
| `bench.ps1 -Fullscreen` | startup 349 ms · idle CPU 16 ms / 10 s · 25.5 MB private · 0 MB VRAM |
| `soak.ps1 -Cycles 150` (now also opens the menu and the app list every cycle, ~300 process handles each time) | private 27.4 → 27.2 MB, handles 300 → 289, GDI 17 → 17 |
| `focus-soak.ps1 -Switches 30` | threads 12 → 12, handles 299 → 299 |
| `custom-entry-test.ps1` | PASS |
| `game-launch-test.ps1` (Deadlock, XBGG alone) | XBGG 16 ms CPU over 30 s of play, back in front after exit |
| `update-test.ps1` (through the new menu) | PASS |
| **Real update, published binaries only**: v0.1.1 downloaded from GitHub, F2 + Enter → exited 2 s later, restarted as v0.2.0; installed SHA-256 equals the published v0.2.0 checksum, no leftover files | PASS |

## Test environment

| | |
|---|---|
| OS | Windows 11 Home (build 26320) |
| CPU | Intel Core i5, 12th gen (laptop) |
| GPU | NVIDIA GeForce RTX 3050 Laptop + Intel Iris Xe (hybrid) |
| RAM | 16 GB |
| Display | 1920×1080 |
| Antivirus | Microsoft Defender, real-time protection on (signatures 1.463.373.0) |
| Library | 7 Steam games (Deadlock, Lossless Scaling, Monster Hunter Wilds, Wallpaper Engine, Red Dead Redemption 2, Rainbow Six Siege, R.E.P.O.) |

All scripts are in [`tools/`](../tools) and can be re-run.

## 1. Unit tests — `cargo test --release`

17 passed, 0 failed (+1 ignored micro-benchmark).

| Area | Tests |
|---|---|
| VDF parser | library folders, unterminated input |
| Library | add/remove custom entry persists, rejects unsupported files, corrupt config moved aside |
| Controller | button mapping, d-pad repeat, stick hysteresis, disconnect, edge detection |
| Artwork | rounded corners, fade curve, baked image exact size & opaque |
| Updater | version parsing/ordering, newer-release selection, checksum parsing, exe swap + backup |

`cargo clippy --release --all-targets`: no warnings.

## 2. Resource benchmark — `tools/bench.ps1 -Fullscreen`

Three runs on the final build (12 navigation steps with animation per run).

| Metric | Run 1 | Run 2 | Run 3 |
|---|---|---|---|
| Startup to window (ms) | 2377¹ | 352 | 294 |
| Idle CPU, XBGG in front (ms / 10 s) | 0 | 0 | 0 |
| CPU with another window in front (ms / 10 s) | 0 | 0 | 16 |
| CPU for 12 animated moves (ms) | 1078 | 1203 | 1078 |
| Private memory (MB) | 26.1 | 25.8 | 25.9 |
| GPU dedicated / shared (MB) | 0 / 0 | 0 / 0 | 0 / 0 |
| Threads / handles | 12 / 292 | 12 / 292 | 12 / 292 |

¹ First launch of a freshly built exe includes Defender's on-access scan.

Windows reports CPU time in 15.6 ms ticks, so 0–16 ms means "below measurement resolution".

## 3. Leak tests

### 3a. UI soak — `tools/soak.ps1`
Each cycle: jump to the end and back, move 3 tiles (background art reloads),
open/cancel the exit dialog, open/cancel the remove dialog; full rescan every
5th cycle.

| Run | Cycles | Private MB (early → late, max) | Handles | GDI | USER | Threads |
|---|---|---|---|---|---|---|
| 1 | 200 | 26.1 → 27.1, max 27.2 | 290 → 279 | 17 → 17 | 20 → 19 | 12 → 5 |
| 2 | **500** | **26.3 → 26.2, max 26.4** | 289 → 279 | 17 → 17 | 20 → 19 | 10 → 8 |

Run 1 moved in small steps and then stayed flat for 60+ cycles (allocator
reuse of the 8 MB background buffers); the 500-cycle run confirms there is no
growth. Threads drop because the controller poller is idle when not in front.

### 3b. Focus switching — `tools/focus-soak.ps1 -Switches 40`
Foreground alternated between XBGG and a stand-in game window 40 times.

| | Start | End |
|---|---|---|
| Threads | 12 | 8 |
| Handles | 293 | 286 |
| Private memory (MB) | 19.7 | 19.6 |
| XBGG reached foreground | | 40/40 |
| CPU with game in front afterwards | | 16 ms / 20 s |

## 4. End-to-end: real game — `tools/game-launch-test.ps1 -GameProcess deadlock -Title Deadlock`

Run three times during development, then on the v0.1.0 and v0.1.1 release
binaries. Enter is pressed twice to check that a double press doesn't start a
second copy.

| Check | v0.1.0 | v0.1.1 |
|---|---|---|
| Game process started | 3.3 s after Enter | 2.3 s after Enter |
| Game instances running | 1 (double press ignored) | 1 |
| Game window took the foreground over full-screen XBGG | 14.4 s (game load time) | 12.5 s |
| **XBGG CPU while the game ran (45 s)** | **16 ms — 0.04 % of one core** | **16 ms — 0.04 %** |
| XBGG private memory / GPU memory during play | 26.6 MB / 0 MB | 26.1 MB / 0 MB |
| XBGG threads during play | 10 (controller polling stopped) | 10 |
| Deadlock for reference | 5.8 GB private, 1.5 GB VRAM | 5.8 GB, 1.5 GB VRAM |
| Game closed | ✅ (WM_CLOSE; see limitations) |
| XBGG back in the foreground after the game exits | ✅ |
| Deadlock listed first afterwards (recently played) | ✅ |

History of the "XBGG CPU while the game ran" metric across fixes:

| Build | CPU during 45 s of Deadlock |
|---|---|
| gilrs XInput backend (leaking poller) | 516 ms (1.15 % of a core) |
| Final | 16 ms (0.04 % of a core) |

## 5. End-to-end: add / launch / remove — `tools/custom-entry-test.ps1`

Drives the real Windows file picker, adds `winver.exe`, launches it, removes it
and restores the user's `library.json`.

| Check | Result |
|---|---|
| File picker opens on Insert | ✅ |
| XBGG ignores Enter/→ while the picker is open | ✅ |
| Entry saved to `library.json` and focused | ✅ |
| Enter launches it exactly once | ✅ |
| Delete → confirm removes it from `library.json` | ✅ |
| **Result** | **PASS** |

## 6. Other checks

| Check | Result |
|---|---|
| Second `xbgg.exe` focuses the running one and exits | ✅ |
| `--list`, `--version`, `--help` print to a redirected stdout | ✅ |
| Unknown option exits with code 2 and prints usage | ✅ |
| `--check-update` with no published release reports "Up to date" | ✅ |
| Rendering at 1920×1080 full screen (screenshot review) | ✅ |
| Embedded manifest is the one Windows loads (`asInvoker`, PerMonitorV2 DPI) | ✅ |
| Version info (`FileDescription`, `ProductVersion` 0.1.0, icon) | ✅ |

### In-app update, end to end — `tools/update-test.ps1`

A copy of `xbgg.exe` with 16 extra bytes appended (so its hash differs) runs
as a pretend older version (`XBGG_PRETEND_VERSION=0.0.1`) against the
published GitHub release.

| Check | Result |
|---|---|
| `--check-update` → "Update available: 0.0.1 -> 0.1.0" | ✅ |
| "Update available" badge shown in the top bar | ✅ |
| F2 opens the update dialog, Enter confirms | ✅ |
| Old process exits, new one starts (single-instance hand-over) | ✅ 4 s after confirming |
| Installed binary's SHA-256 equals the release's `xbgg.exe.sha256` | ✅ |
| No `xbgg.exe.old` / `xbgg.exe.new` left behind | ✅ |
| **Result** | **PASS** |

**Real-world update, released binaries only:** the published v0.1.0 `xbgg.exe`
was downloaded from GitHub and run unmodified. It showed "Update v0.1.1
available"; clicking the badge opened "Update to v0.1.1?", Enter confirmed,
the process exited 3 s later and restarted as v0.1.1. The installed file's
SHA-256 (`c0498094…a0e5db`) equals the published v0.1.1 checksum and file
version reads 0.1.1. **PASS.**

## 7. Antivirus

Microsoft Defender (engine 1.1.26110.3, signatures 1.463.373.0), real-time
protection on for the whole test session.

| Check | Result |
|---|---|
| On-demand scan of `xbgg.exe` v0.1.0 and v0.1.1 (release assets) | No threats |
| On-demand scan of `XBGG-v0.1.0-windows-x64.zip` and `XBGG-v0.1.1-windows-x64.zip` | No threats |
| Detections or Defender events mentioning `xbgg` during all builds and test runs | None |

Design choices that keep heuristics quiet: runs `asInvoker` (never asks for
admin), full version information and icon embedded, TLS through Windows'
own SChannel and certificate store, XBGG itself never spawns shells, script
interpreters or downloaders (only what the user launches), updates are user-confirmed and checksum-verified, no code injection
or memory access into games (anti-cheat safe).

## 8. Bugs found and fixed during testing

| # | Severity | Found by | Problem | Fix |
|---|---|---|---|---|
| 1 | **Critical** | `custom-entry-test.ps1` | Adding a game crashed XBGG: `RefCell` borrowed mutably and immutably in one statement (release build aborts silently). | Split into separate statements; added `%APPDATA%\XBGG\crash.log`. |
| 2 | **High** | `focus-soak.ps1` | The gilrs XInput backend never stops its 10 ms polling thread; re-creating it on every focus change leaked a thread each time (14 → 90 threads, 11% of a core after 40 switches). | Replaced gilrs with a small XInput poller that runs only while XBGG is in front. |
| 3 | **High** | GPU counters / `nvidia-smi` | OpenGL renderer ran on the RTX 3050 and held 47.5 MB of VRAM that games should get. | Switched to Slint's software renderer: 0 MB VRAM, private memory 125 → 26 MB. |
| 4 | Medium | Thread profiling | gilrs' default WGI backend polled WinRT every 8 ms forever (~2.5% of a core idle). | Own XInput poller (see #2). |
| 5 | Medium | `bench.ps1` | Each selection change spent ~380 ms re-scaling 3840 px background art. | Single-pass fixed-point bake at window size: 20 ms. |
| 6 | Medium | `bench.ps1` | Software renderer rescaled covers and clipped rounded corners every animation frame (100% of a core while scrolling). | Covers decoded once at exact tile size with corners baked in; ~4× less CPU per move. |
| 7 | Low | `--check-update` | With no published release yet, GitHub's 404 was reported as an error. | 404 means "no update". |
| 8 | Low | Review | Rescan warnings accumulated in memory and were never shown. | Shown in the status line, then cleared. |
| 9 | Low | Review | Starting XBGG twice opened two full-screen shells. | Single-instance lock; restart after update waits for the old process. |
| 10 | Medium | `update-test.ps1` (shipped in v0.1.0) | The `U` shortcut never fired on a Thai keyboard layout: the key types "อ", and Slint matches typed text. | **v0.1.1**: shortcut is F2 (layout-independent); `U` kept for Latin layouts. |

## 9. Known limitations

- The executable is not code-signed: SmartScreen shows "Windows protected your
  PC" for downloaded copies until the app gains reputation.
- Synthetic Alt+F4 from the test script did not close Deadlock; the test falls
  back to a normal window close (WM_CLOSE), which worked. Not an XBGG issue.
- Non-XInput controllers need Steam Input or DS4Windows.
- Hidden games can only be un-hidden by editing `library.json`.
