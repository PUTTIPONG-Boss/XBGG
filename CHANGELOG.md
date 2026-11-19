# Changelog

All notable changes to XBGG are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

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

[0.1.0]: https://github.com/PUTTIPONG-Boss/XBGG/releases/tag/v0.1.0
