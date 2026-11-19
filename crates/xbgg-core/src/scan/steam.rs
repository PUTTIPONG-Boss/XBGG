//! Steam: reads `libraryfolders.vdf` and every `appmanifest_*.acf`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::model::{Game, LaunchTarget, Source};
use crate::vdf;

/// App ids that are tools/runtimes rather than games.
const NON_GAME_APP_IDS: &[&str] = &[
    "228980",  // Steamworks Common Redistributables
    "1070560", // Steam Linux Runtime
    "1391110", // Steam Linux Runtime - Soldier
    "1628350", // Steam Linux Runtime - Sniper
    "1493710", // Proton Experimental
];

const NON_GAME_PREFIXES: &[&str] = &["Proton ", "Steam Linux Runtime", "Steamworks Common"];

/// Steam's "fully installed" state flag.
const STATE_FULLY_INSTALLED: u32 = 4;

pub fn scan() -> Result<Vec<Game>> {
    let Some(root) = steam_root() else {
        return Ok(Vec::new());
    };

    let mut libraries = vec![root.clone()];
    let lf_path = root.join("steamapps").join("libraryfolders.vdf");
    if let Ok(text) = fs::read_to_string(&lf_path) {
        let doc = vdf::parse(&text)
            .map_err(|e| anyhow::anyhow!(e))
            .with_context(|| format!("parsing {}", lf_path.display()))?;
        if let Some(lf) = doc.get("libraryfolders") {
            for (_, folder) in lf.children() {
                if let Some(p) = folder.str("path") {
                    let p = PathBuf::from(p);
                    if !libraries.iter().any(|l| same_path(l, &p)) {
                        libraries.push(p);
                    }
                }
            }
        }
    }

    let cache = root.join("appcache").join("librarycache");
    let mut games = Vec::new();
    for lib in &libraries {
        let Ok(entries) = fs::read_dir(lib.join("steamapps")) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !(name.starts_with("appmanifest_") && name.ends_with(".acf")) {
                continue;
            }
            // A single malformed manifest should not hide the rest of the library.
            if let Some(game) = read_manifest(&entry.path(), &cache) {
                if !games.iter().any(|g: &Game| g.id == game.id) {
                    games.push(game);
                }
            }
        }
    }
    Ok(games)
}

fn read_manifest(path: &Path, cache: &Path) -> Option<Game> {
    let text = fs::read_to_string(path).ok()?;
    let doc = vdf::parse(&text).ok()?;
    let app = doc.get("AppState")?;
    let appid = app.str("appid")?.to_string();
    let title = app.str("name").unwrap_or(&appid).to_string();

    let flags: u32 = app.str("StateFlags").and_then(|s| s.parse().ok()).unwrap_or(0);
    if flags & STATE_FULLY_INSTALLED == 0 {
        return None;
    }
    if NON_GAME_APP_IDS.contains(&appid.as_str())
        || NON_GAME_PREFIXES.iter().any(|p| title.starts_with(p))
    {
        return None;
    }

    let last_played = app
        .str("LastPlayed")
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|&t| t > 0);

    Some(Game {
        id: format!("steam:{appid}"),
        title,
        source: Source::Steam,
        launch: LaunchTarget::Uri(format!("steam://rungameid/{appid}")),
        cover: find_art(cache, &appid, &["library_600x900.jpg", "library_capsule.jpg"]),
        hero: find_art(cache, &appid, &["library_hero.jpg"]),
        last_played,
    })
}

/// Finds cached artwork. Supports both the current layout
/// (`librarycache/<appid>/<hash>/<file>`) and the older flat one
/// (`librarycache/<appid>_<file>`).
fn find_art(cache: &Path, appid: &str, names: &[&str]) -> Option<PathBuf> {
    let dir = cache.join(appid);
    for name in names {
        let direct = dir.join(name);
        if direct.is_file() {
            return Some(direct);
        }
        if let Ok(entries) = fs::read_dir(&dir) {
            for e in entries.flatten() {
                let candidate = e.path().join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
        let flat = cache.join(format!("{appid}_{name}"));
        if flat.is_file() {
            return Some(flat);
        }
    }
    None
}

fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase();
    norm(a) == norm(b)
}

#[cfg(windows)]
fn steam_root() -> Option<PathBuf> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    let from_hkcu = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Valve\Steam")
        .and_then(|k| k.get_value::<String, _>("SteamPath"))
        .ok();
    let from_hklm = || {
        RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey(r"SOFTWARE\WOW6432Node\Valve\Steam")
            .and_then(|k| k.get_value::<String, _>("InstallPath"))
            .ok()
    };
    from_hkcu
        .or_else(from_hklm)
        .map(|p| PathBuf::from(p.replace('/', "\\")))
        .or_else(|| Some(PathBuf::from(r"C:\Program Files (x86)\Steam")))
        .filter(|p| p.join("steamapps").is_dir())
}

#[cfg(not(windows))]
fn steam_root() -> Option<PathBuf> {
    None
}
