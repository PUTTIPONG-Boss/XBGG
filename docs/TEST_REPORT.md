# XBGG v0.1.0 — Test Report

Date: 2026-11-20 · Build: `release` profile (thin LTO, `opt-level = 3`), Rust 1.99.0,
`x86_64-pc-windows-gnu`.

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

Run three times during development and once on the final release binary
(`dist\xbgg.exe`, SHA-256 `6be13711…41caec`). Enter is pressed twice to check
that a double press doesn't start a second copy.

| Check | Final run |
|---|---|
| Game process started | 3.3 s after Enter |
| Game instances running | 1 (double press ignored) |
| Game window took the foreground over full-screen XBGG | 14.4 s (game load time) |
| **XBGG CPU while the game ran (45 s)** | **16 ms total — 0.04 % of one core** |
| XBGG private memory / GPU memory during play | 26.6 MB / 0 MB |
| XBGG threads during play | 10 (controller polling stopped) |
| Deadlock for reference | 5.8 GB private, 1.5 GB VRAM |
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

### In-app update, end to end

Verified against the published v0.1.0 release (see below).

## 7. Antivirus

Microsoft Defender (engine 1.1.26110.3, signatures 1.463.373.0), real-time
protection on for the whole test session.

| Check | Result |
|---|---|
| On-demand scan of `xbgg.exe` (release asset) | No threats |
| On-demand scan of `XBGG-v0.1.0-windows-x64.zip` | No threats |
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

## 9. Known limitations

- The executable is not code-signed: SmartScreen shows "Windows protected your
  PC" for downloaded copies until the app gains reputation.
- Synthetic Alt+F4 from the test script did not close Deadlock; the test falls
  back to a normal window close (WM_CLOSE), which worked. Not an XBGG issue.
- Non-XInput controllers need Steam Input or DS4Windows.
- Hidden games can only be un-hidden by editing `library.json`.
