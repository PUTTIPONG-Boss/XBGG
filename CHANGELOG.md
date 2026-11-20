# Changelog

All notable changes to XBGG are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [0.5.0] - 2026-11-20

### Added
- **Shell mode** (menu):
  - **start with windows** — registers XBGG to start at sign-in (current user
    only, `HKCU\…\Run`).
  - **hide desktop while open** — closes the Windows desktop and taskbar
    (Explorer exits cleanly, like *Exit Explorer*) while XBGG is open and
    brings it back when XBGG closes. If File Explorer windows are open in the
    desktop process it refuses and says so. If Explorer lingers it is ended,
    and on the way back a stuck or missing taskbar is replaced by a fresh
    one. Crash-safe: recorded before acting, restored by the guard process
    or at the next start; `xbgg --restore` restores it by hand.
- **Pause Windows services while playing** (Boost, opt-in): Windows Search
  indexing, SysMain, Telemetry and Delivery Optimization are stopped and
  temporarily disabled (so they can't restart themselves), and their exact
  start types are restored afterwards. Needs a one-time administrator setup
  from the menu: a helper copy in `Program Files\XBGG` plus two Task
  Scheduler tasks with fixed arguments, so no prompt appears during play.
  The menu offers to update the helper when XBGG is newer, and to remove it.

### Security
- The elevated helper accepts no input from the user side: a hard-coded
  service allowlist, state kept next to the helper in an administrators-only
  folder, `schtasks.exe` called by full path.

## [0.4.0] - 2026-11-20

### Added
- **Two layouts**, switchable in *Menu → layout* (saved):
  - **terminal** (default): library list + details pane, monospace.
  - **console**: the cover carousel with the focused game's background art,
    now with square tiles, a bordered source label and no gradients.
- Square checkboxes in the menu and app list in the console layout.

### Changed
- Everything is square: no rounded corners or pill shapes anywhere, and the
  controller keys are square blocks in both layouts.

## [0.3.0] - 2026-11-20

A new, terminal-style interface.

### Changed
- **Terminal UI**: a library list (title / source / last played) and a
  details pane with the cover, framed TUI panels, one monospace face
  (Cascadia Mono, embedded), one accent colour, inverse-video selection, an
  htop-style key bar. Menu, "apps to freeze" and confirmations are framed
  dialogs. No carousel, background art, gradients, rounded corners or
  animation.
- ↑/↓ (d-pad, stick, arrow keys) move through the library; ←/→ still work;
  Page Up/Down jump a screen.

### Performance
- Private memory 25.5 → **13.7 MB**: only the focused game's cover is decoded.
- CPU to move through 12 entries ~1100 → **~190 ms** (nothing animates).

## [0.2.0] - 2026-11-20

Boost mode: XBGG now frees resources for the game while you play.

### Added
- **Boost while playing** (on by default). When a game launched from XBGG
  starts, XBGG:
  - **freezes chosen background apps** — suspends them and releases their RAM
    to the game. Nothing is closed and no work is lost; they resume exactly
    where they were when the game exits;
  - switches Windows to the **Best performance** power mode (and back);
  - optionally raises the game to **High priority** (off by default), and
    re-applies it if the game resets its own priority while starting up.
- **Apps to freeze** screen: every app with a window plus common heavy apps,
  with live process count and memory use. Browsers, Wallpaper Engine, Teams,
  Slack, OneDrive and similar are on by default; Discord, Spotify and OBS are
  offered but off. Windows, game launchers and anti-cheat can never be frozen.
- Game detection: the boost ends ~2 s after the game exits (by its process, or
  by its install folder for Steam/Epic/GOG games).
- **Crash safety**: every change is journaled before it is made; a guard
  process restores everything if XBGG is killed; leftovers from a power cut
  are restored at the next start. `xbgg --restore` does it by hand.
- **Menu** (controller Menu button / F2): boost settings, apps to freeze,
  install update, rescan, exit. Up/Down navigation on d-pad, left stick and
  arrow keys.
- BOOST indicator in the top bar with what was freed, and a summary when the
  session ends.

### Changed
- The update prompt moved into the menu (top row when an update exists).

## [0.1.1] - 2026-11-20

### Fixed
- The "install update" keyboard shortcut did nothing on non-Latin keyboard
  layouts (e.g. Thai, where the U key types "อ"). It is now **F2**, which works
  with every layout; `U` still works on Latin layouts. The controller's Menu
  button and clicking the update badge were not affected.

## [0.1.0] - 2026-11-20

First public release: the game library shell.

### Added
- Full-screen, controller-first library of installed games from **Steam**
  (all library folders, local cover and background art), **Epic Games** and **GOG**.
- **Add game or app**: any `.exe`, `.lnk`, `.url`, `.bat` or `.cmd`.
- Remove custom entries / hide scanned games (files are never touched).
- Recently played entries first.
- Xbox-style controller (XInput), keyboard and mouse input.
- **In-app updates** from GitHub Releases with SHA-256 verification, user
  confirmation and automatic rollback if the swap fails.
- Single instance: launching XBGG again focuses the running window.
- Command-line modes: `--list`, `--check-update`, `--version`, `--help`,
  `--windowed`, `--no-update`.
- Crash log at `%APPDATA%\XBGG\crash.log`.
- Embedded version information, icon and `asInvoker` manifest.

### Performance
- CPU-only software renderer: **0 MB of VRAM**, never wakes a laptop's dGPU.
- ~26 MB private memory; ~0% CPU while a game is in the foreground.
- Artwork prepared once at exact on-screen size; background art baked in a
  single fixed-point pass (380 ms → 20 ms per image).
- Controller polling runs only while XBGG is in front.

See [docs/TEST_REPORT.md](docs/TEST_REPORT.md) for measurements.

[0.5.0]: https://github.com/PUTTIPONG-Boss/XBGG/releases/tag/v0.5.0
[0.4.0]: https://github.com/PUTTIPONG-Boss/XBGG/releases/tag/v0.4.0
[0.3.0]: https://github.com/PUTTIPONG-Boss/XBGG/releases/tag/v0.3.0
[0.2.0]: https://github.com/PUTTIPONG-Boss/XBGG/releases/tag/v0.2.0
[0.1.1]: https://github.com/PUTTIPONG-Boss/XBGG/releases/tag/v0.1.1
[0.1.0]: https://github.com/PUTTIPONG-Boss/XBGG/releases/tag/v0.1.0
