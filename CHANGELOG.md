# Changelog

All notable changes to XBGG are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

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

[0.2.0]: https://github.com/PUTTIPONG-Boss/XBGG/releases/tag/v0.2.0
[0.1.1]: https://github.com/PUTTIPONG-Boss/XBGG/releases/tag/v0.1.1
[0.1.0]: https://github.com/PUTTIPONG-Boss/XBGG/releases/tag/v0.1.0
