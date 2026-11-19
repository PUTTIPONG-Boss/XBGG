//! GOG: reads the per-game registry keys written by GOG installers / Galaxy.

use anyhow::Result;

use crate::model::Game;

#[cfg(windows)]
pub fn scan() -> Result<Vec<Game>> {
    use std::path::PathBuf;

    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    use crate::model::{LaunchTarget, Source};

    let Ok(root) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(r"SOFTWARE\WOW6432Node\GOG.com\Games") else {
        return Ok(Vec::new());
    };

    let mut games = Vec::new();
    for id in root.enum_keys().flatten() {
        let Ok(key) = root.open_subkey(&id) else { continue };
        let get = |name: &str| key.get_value::<String, _>(name).ok().filter(|s| !s.is_empty());

        // DLC entries depend on a base game; only list the base game.
        if get("dependsOn").is_some() {
            continue;
        }
        let Some(exe) = get("exe").map(PathBuf::from) else { continue };
        if !exe.is_file() {
            continue;
        }
        let title = get("gameName").or_else(|| get("startMenu")).unwrap_or_else(|| id.clone());
        let args = get("launchParam")
            .map(|p| p.split_whitespace().map(str::to_string).collect())
            .unwrap_or_default();
        let working_dir = get("workingDir").or_else(|| get("path")).map(PathBuf::from);

        games.push(Game {
            id: format!("gog:{id}"),
            title,
            source: Source::Gog,
            launch: LaunchTarget::Exe { path: exe, args, working_dir },
            cover: None,
            hero: None,
            last_played: None,
        });
    }
    Ok(games)
}

#[cfg(not(windows))]
pub fn scan() -> Result<Vec<Game>> {
    Ok(Vec::new())
}
