//! XBGG core: finds installed games, keeps the user's library and launches entries.

pub mod boost;
pub mod launch;
pub mod model;
pub mod scan;
#[cfg(windows)]
pub mod shell;
pub mod store;
pub mod vdf;

use std::cmp::Reverse;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Result};

pub use model::{Game, LaunchTarget, Source};
use store::{Config, CustomEntry};

pub fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Extensions accepted by "Add game or app".
pub const ADDABLE_EXTENSIONS: &[&str] = &["exe", "lnk", "url", "bat", "cmd"];

pub struct Library {
    games: Vec<Game>,
    config: Config,
    config_path: Option<PathBuf>,
    /// Problems found while loading/scanning, for display to the user.
    pub warnings: Vec<String>,
}

impl Library {
    pub fn load() -> Self {
        Self::load_from(store::default_path())
    }

    pub fn load_from(config_path: Option<PathBuf>) -> Self {
        let mut warnings = Vec::new();
        let config = match &config_path {
            Some(p) => {
                let (cfg, warn) = store::load(p);
                warnings.extend(warn);
                cfg
            }
            None => {
                warnings.push("No config directory available; changes will not be saved".into());
                Config::default()
            }
        };
        let mut lib = Library { games: Vec::new(), config, config_path, warnings };
        lib.rescan();
        lib
    }

    pub fn games(&self) -> &[Game] {
        &self.games
    }

    /// Returns and clears the problems collected since the last call.
    pub fn take_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.warnings)
    }

    /// Re-reads every store and rebuilds the visible list.
    pub fn rescan(&mut self) {
        let (scanned, warnings) = scan::scan_all();
        self.warnings.extend(warnings);

        let mut games: Vec<Game> = scanned
            .into_iter()
            .filter(|g| !self.config.hidden.contains(&g.id))
            .collect();
        games.extend(self.config.custom.iter().map(custom_to_game));

        for g in &mut games {
            if let Some(&t) = self.config.last_played.get(&g.id) {
                g.last_played = Some(g.last_played.map_or(t, |s| s.max(t)));
            }
        }
        sort_games(&mut games);
        self.games = games;
    }

    /// Adds an executable, shortcut or script. Returns the new entry's index.
    pub fn add_custom(&mut self, path: &Path) -> Result<usize> {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        if !ADDABLE_EXTENSIONS.contains(&ext.as_str()) {
            bail!("Unsupported file type: .{ext}");
        }
        if !path.is_file() {
            bail!("File not found: {}", path.display());
        }
        if let Some(existing) = self.config.custom.iter().find(|c| c.path == path) {
            let id = existing.id.clone();
            return self.index_of(&id).ok_or_else(|| anyhow::anyhow!("Entry exists but is not listed"));
        }

        let title = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let id = format!(
            "custom:{:x}",
            SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
        );
        let entry = CustomEntry { id: id.clone(), title, path: path.to_path_buf(), args: Vec::new() };

        self.config.custom.push(entry.clone());
        if let Err(e) = self.save() {
            self.config.custom.pop();
            return Err(e);
        }
        self.games.push(custom_to_game(&entry));
        sort_games(&mut self.games);
        self.index_of(&id).ok_or_else(|| anyhow::anyhow!("Added entry missing from library"))
    }

    /// Removes a custom entry, or hides a scanned game. Files are never touched.
    pub fn remove(&mut self, index: usize) -> Result<()> {
        let Some(game) = self.games.get(index) else { bail!("No entry at index {index}") };
        let id = game.id.clone();
        if game.source == Source::Custom {
            self.config.custom.retain(|c| c.id != id);
        } else if !self.config.hidden.contains(&id) {
            self.config.hidden.push(id.clone());
        }
        self.config.last_played.remove(&id);
        self.save()?;
        self.games.remove(index);
        Ok(())
    }

    /// Starts the entry and records the launch time. Also returns the process
    /// ID when XBGG started the process itself.
    pub fn launch(&mut self, index: usize) -> Result<(&Game, Option<u32>)> {
        let Some(game) = self.games.get(index) else { bail!("No entry at index {index}") };
        let pid = launch::launch(&game.launch)?;

        let now = now_unix();
        let id = game.id.clone();
        self.config.last_played.insert(id, now);
        if let Err(e) = self.save() {
            self.warnings.push(format!("Could not save launch history: {e:#}"));
        }
        let game = &mut self.games[index];
        game.last_played = Some(now);
        Ok((game, pid))
    }

    pub fn boost(&self) -> &boost::BoostConfig {
        &self.config.boost
    }

    /// Changes the boost settings and saves them.
    pub fn update_boost(&mut self, change: impl FnOnce(&mut boost::BoostConfig)) -> Result<()> {
        let before = self.config.boost.clone();
        change(&mut self.config.boost);
        if let Err(e) = self.save() {
            self.config.boost = before;
            return Err(e);
        }
        Ok(())
    }

    pub fn ui(&self) -> &store::UiSettings {
        &self.config.ui
    }

    /// Turns "hide the desktop while XBGG is open" on or off and saves it.
    pub fn set_hide_desktop(&mut self, on: bool) -> Result<()> {
        let before = self.config.ui.hide_desktop;
        self.config.ui.hide_desktop = on;
        if let Err(e) = self.save() {
            self.config.ui.hide_desktop = before;
            return Err(e);
        }
        Ok(())
    }

    /// Switches between the terminal and console layouts and saves it.
    pub fn set_console_layout(&mut self, console: bool) -> Result<()> {
        let before = self.config.ui.clone();
        self.config.ui.layout = if console { store::LAYOUT_CONSOLE } else { store::LAYOUT_TERMINAL }.into();
        if let Err(e) = self.save() {
            self.config.ui = before;
            return Err(e);
        }
        Ok(())
    }

    /// Re-sorts after launches so recently played entries come first.
    pub fn resort(&mut self) {
        sort_games(&mut self.games);
    }

    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.games.iter().position(|g| g.id == id)
    }

    fn save(&self) -> Result<()> {
        match &self.config_path {
            Some(p) => store::save(p, &self.config),
            None => Ok(()),
        }
    }
}

fn custom_to_game(c: &CustomEntry) -> Game {
    let ext = c.path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let is_exe = ext == "exe";
    let launch = if is_exe {
        LaunchTarget::Exe { path: c.path.clone(), args: c.args.clone(), working_dir: None }
    } else {
        LaunchTarget::Shell(c.path.clone())
    };
    Game {
        id: c.id.clone(),
        title: c.title.clone(),
        source: Source::Custom,
        launch,
        cover: None,
        hero: None,
        last_played: None,
        // Shortcuts point elsewhere; only a direct .exe tells us the folder.
        install_dir: is_exe.then(|| c.path.parent().map(|p| p.to_path_buf())).flatten(),
    }
}

/// Recently played first, then alphabetical.
fn sort_games(games: &mut [Game]) {
    games.sort_by_cached_key(|g| (Reverse(g.last_played.unwrap_or(0)), g.title.to_lowercase()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_config(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xbgg-test-{name}-{}", now_unix()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("library.json")
    }

    #[test]
    fn add_and_remove_custom_entry_persists() {
        let cfg = temp_config("custom");
        let exe = std::env::current_exe().unwrap();

        let mut lib = Library::load_from(Some(cfg.clone()));
        let before = lib.games().len();
        let idx = lib.add_custom(&exe).unwrap();
        assert_eq!(lib.games()[idx].source, Source::Custom);

        // Adding the same file twice returns the existing entry.
        assert_eq!(lib.add_custom(&exe).unwrap(), idx);

        let reloaded = Library::load_from(Some(cfg.clone()));
        assert_eq!(reloaded.games().len(), before + 1);

        lib.remove(idx).unwrap();
        let reloaded = Library::load_from(Some(cfg.clone()));
        assert_eq!(reloaded.games().len(), before);
        let _ = std::fs::remove_dir_all(cfg.parent().unwrap());
    }

    #[test]
    fn layout_setting_persists_and_tolerates_unknown_values() {
        let cfg = temp_config("layout");
        let mut lib = Library::load_from(Some(cfg.clone()));
        assert!(!lib.ui().is_console(), "terminal is the default");
        lib.set_console_layout(true).unwrap();
        assert!(Library::load_from(Some(cfg.clone())).ui().is_console());

        // A value written by some future version must not break the file.
        let text = std::fs::read_to_string(&cfg).unwrap().replace("\"console\"", "\"hologram\"");
        std::fs::write(&cfg, text).unwrap();
        let lib = Library::load_from(Some(cfg.clone()));
        assert!(lib.warnings.is_empty(), "{:?}", lib.warnings);
        assert!(!lib.ui().is_console());
        let _ = std::fs::remove_dir_all(cfg.parent().unwrap());
    }

    #[test]
    fn rejects_unsupported_files() {
        let cfg = temp_config("reject");
        let mut lib = Library::load_from(Some(cfg.clone()));
        assert!(lib.add_custom(Path::new("C:/nope/readme.txt")).is_err());
        let _ = std::fs::remove_dir_all(cfg.parent().unwrap());
    }

    #[test]
    fn corrupt_config_is_moved_aside() {
        let cfg = temp_config("corrupt");
        std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        std::fs::write(&cfg, "{ not json").unwrap();
        let lib = Library::load_from(Some(cfg.clone()));
        assert!(lib.warnings.iter().any(|w| w.contains("invalid")));
        assert!(!cfg.exists());
        let _ = std::fs::remove_dir_all(cfg.parent().unwrap());
    }
}
