//! Persistent user data: custom entries, hidden ids and launch history.
//! Stored as JSON at `%APPDATA%\XBGG\library.json`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomEntry {
    pub id: String,
    pub title: String,
    pub path: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub custom: Vec<CustomEntry>,
    /// Ids of scanned games the user chose to hide.
    pub hidden: Vec<String>,
    /// Unix seconds of the last launch from XBGG, by game id.
    pub last_played: BTreeMap<String, u64>,
}

pub fn default_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("XBGG").join("library.json"))
}

/// Loads the config. A corrupt file is moved aside (never silently
/// overwritten) and a warning is returned alongside an empty config.
pub fn load(path: &Path) -> (Config, Option<String>) {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Config::default(), None),
        Err(e) => return (Config::default(), Some(format!("Could not read {}: {e}", path.display()))),
    };
    match serde_json::from_str(&text) {
        Ok(cfg) => (cfg, None),
        Err(e) => {
            let backup = path.with_extension(format!("corrupt-{}.json", crate::now_unix()));
            let moved = fs::rename(path, &backup).is_ok();
            let note = if moved {
                format!("Library file was invalid ({e}); kept a copy at {}", backup.display())
            } else {
                format!("Library file was invalid ({e})")
            };
            (Config::default(), Some(note))
        }
    }
}

/// Writes atomically: temp file first, then rename over the original.
pub fn save(path: &Path, cfg: &Config) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(cfg)?;
    fs::write(&tmp, json).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}
