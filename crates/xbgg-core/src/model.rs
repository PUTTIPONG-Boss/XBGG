use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    Steam,
    Epic,
    Gog,
    Custom,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Steam => "Steam",
            Source::Epic => "Epic Games",
            Source::Gog => "GOG",
            Source::Custom => "Custom",
        }
    }
}

/// How a library entry is started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LaunchTarget {
    /// A protocol URI handled by a launcher (e.g. `steam://rungameid/123`).
    Uri(String),
    /// An executable started directly.
    Exe {
        path: PathBuf,
        args: Vec<String>,
        working_dir: Option<PathBuf>,
    },
    /// Anything Windows can open (shortcuts, `.url`, `.bat`, ...).
    Shell(PathBuf),
}

#[derive(Debug, Clone)]
pub struct Game {
    /// Stable id, e.g. `steam:359550`, `epic:Fortnite`, `custom:1a2b3c`.
    pub id: String,
    pub title: String,
    pub source: Source,
    pub launch: LaunchTarget,
    /// Portrait cover art (2:3), if available locally.
    pub cover: Option<PathBuf>,
    /// Wide background art, if available locally.
    pub hero: Option<PathBuf>,
    /// Unix seconds of the last launch known to either the store or XBGG.
    pub last_played: Option<u64>,
    /// Folder the game runs from; used to recognise its process for Boost.
    pub install_dir: Option<PathBuf>,
}
