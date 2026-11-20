//! Epic Games Launcher: reads the install manifests in ProgramData.

use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use serde::Deserialize;

use crate::model::{Game, LaunchTarget, Source};

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Manifest {
    display_name: Option<String>,
    app_name: Option<String>,
    catalog_namespace: Option<String>,
    catalog_item_id: Option<String>,
    install_location: Option<String>,
    #[serde(default)]
    app_categories: Vec<String>,
    #[serde(default, rename = "bIsIncompleteInstall")]
    is_incomplete_install: bool,
}

pub fn scan() -> Result<Vec<Game>> {
    let base = std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
    let dir = base.join(r"Epic\EpicGamesLauncher\Data\Manifests");
    let Ok(entries) = fs::read_dir(&dir) else {
        return Ok(Vec::new());
    };

    let mut games = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("item") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else { continue };
        let Ok(m) = serde_json::from_str::<Manifest>(&text) else { continue };

        if m.is_incomplete_install {
            continue;
        }
        // Skip engine plugins and other non-game content.
        if !m.app_categories.is_empty() && !m.app_categories.iter().any(|c| c == "games") {
            continue;
        }
        if let Some(loc) = &m.install_location {
            if !PathBuf::from(loc).is_dir() {
                continue;
            }
        }
        let (Some(app), Some(ns), Some(item)) = (m.app_name, m.catalog_namespace, m.catalog_item_id) else {
            continue;
        };

        games.push(Game {
            id: format!("epic:{app}"),
            title: m.display_name.unwrap_or_else(|| app.clone()),
            source: Source::Epic,
            launch: LaunchTarget::Uri(format!(
                "com.epicgames.launcher://apps/{ns}%3A{item}%3A{app}?action=launch&silent=true"
            )),
            cover: None,
            hero: None,
            last_played: None,
            install_dir: m.install_location.map(PathBuf::from),
        });
    }
    Ok(games)
}
