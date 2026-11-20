//! Crash-safe record of every change a boost session makes.
//!
//! The journal is written *before* each change, so after any crash — of XBGG,
//! of Windows, or a power cut — everything can still be put back.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// A process identified by PID *and* creation time, so a recycled PID is
/// never mistaken for the process we changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcRef {
    pub pid: u32,
    pub created: u64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriorityChange {
    pub process: ProcRef,
    pub class_before: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Journal {
    /// XBGG process that owns the session (the guard watches it).
    pub owner_pid: u32,
    pub started_at: u64,
    pub frozen: Vec<ProcRef>,
    /// Power mode before boosting, as a hex GUID number.
    pub power_overlay_before: Option<String>,
    pub priority: Vec<PriorityChange>,
    /// The services helper was asked to pause; ask it to resume on undo.
    pub services_paused: bool,
}

impl Journal {
    pub fn is_empty(&self) -> bool {
        self.frozen.is_empty() && self.power_overlay_before.is_none() && self.priority.is_empty() && !self.services_paused
    }
}

pub fn default_path() -> Option<PathBuf> {
    crate::store::default_path().map(|p| p.with_file_name("boost-journal.json"))
}

pub fn load(path: &Path) -> Option<Journal> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn save(path: &Path, journal: &Journal) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(journal)?).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

pub fn remove(path: &Path) {
    let _ = fs::remove_file(path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_tolerates_missing_fields() {
        let dir = std::env::temp_dir().join(format!("xbgg-journal-{}", std::process::id()));
        let path = dir.join("boost-journal.json");
        let j = Journal {
            owner_pid: 42,
            started_at: 7,
            frozen: vec![ProcRef { pid: 1, created: 2, name: "a.exe".into() }],
            power_overlay_before: Some("0".into()),
            priority: vec![],
            services_paused: true,
        };
        save(&path, &j).unwrap();
        assert_eq!(load(&path), Some(j));
        std::fs::write(&path, r#"{"owner_pid": 5}"#).unwrap();
        let partial = load(&path).unwrap();
        assert_eq!(partial.owner_pid, 5);
        assert!(partial.is_empty());
        remove(&path);
        assert_eq!(load(&path), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
