//! What Boost does, as saved in `library.json` under `"boost"`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BoostConfig {
    /// Master switch: boost while a game launched from XBGG is running.
    pub enabled: bool,
    /// Freeze the apps in `freeze_list` (suspend + release their RAM).
    pub freeze_apps: bool,
    /// Executable names to freeze, lower case (e.g. `chrome.exe`).
    pub freeze_list: Vec<String>,
    /// Switch Windows' power mode to "Best performance".
    pub power_mode: bool,
    /// Raise the game's process priority to "High".
    pub game_priority: bool,
    /// Pause Windows Search, SysMain, Telemetry and Delivery Optimization
    /// (needs the one-time admin setup, see `boost::services`).
    pub pause_services: bool,
}

impl Default for BoostConfig {
    fn default() -> Self {
        BoostConfig {
            enabled: true,
            freeze_apps: true,
            freeze_list: DEFAULT_FREEZE.iter().map(|s| s.to_string()).collect(),
            power_mode: true,
            game_priority: false,
            pause_services: false,
        }
    }
}

/// Frozen by default: heavy apps that rarely matter mid-game.
pub const DEFAULT_FREEZE: &[&str] = &[
    // browsers
    "chrome.exe",
    "msedge.exe",
    "firefox.exe",
    "zen.exe",
    "brave.exe",
    "opera.exe",
    "opera_gx.exe",
    "vivaldi.exe",
    // animated wallpapers
    "wallpaper32.exe",
    "wallpaper64.exe",
    // work chat / sync
    "ms-teams.exe",
    "teams.exe",
    "slack.exe",
    "onedrive.exe",
    "dropbox.exe",
    "googledrivefs.exe",
    // Windows extras
    "widgets.exe",
    "phoneexperiencehost.exe",
    "microsoftstartfeedprovider.exe",
    // Adobe background helpers
    "creative cloud.exe",
    "ccxprocess.exe",
    "adobecollabsync.exe",
];

/// Offered in the app list but off by default (often used while playing).
pub const SUGGESTED: &[&str] = &["discord.exe", "spotify.exe", "obs64.exe"];

/// Never frozen, even if listed: Windows itself, game launchers (the game may
/// need them), anti-cheat, overlays, input/audio, and XBGG.
pub const NEVER_FREEZE: &[&str] = &[
    // Windows
    "system", "smss.exe", "csrss.exe", "wininit.exe", "winlogon.exe", "services.exe", "lsass.exe",
    "svchost.exe", "dwm.exe", "explorer.exe", "sihost.exe", "fontdrvhost.exe", "ctfmon.exe",
    "taskhostw.exe", "runtimebroker.exe", "shellexperiencehost.exe", "startmenuexperiencehost.exe",
    "searchhost.exe", "textinputhost.exe", "audiodg.exe", "conhost.exe", "dllhost.exe",
    "applicationframehost.exe", "securityhealthsystray.exe", "lockapp.exe", "useroobebroker.exe",
    "shellhost.exe", "openconsole.exe", "windowsterminal.exe",
    // game launchers and their helpers
    "steam.exe", "steamwebhelper.exe", "steamservice.exe", "gameoverlayui.exe", "gameoverlayui64.exe",
    "epicgameslauncher.exe", "epicwebhelper.exe", "galaxyclient.exe", "galaxyclient helper.exe",
    "battle.net.exe", "agent.exe", "upc.exe", "ubisoftconnect.exe", "eadesktop.exe", "eabackgroundservice.exe",
    "riotclientservices.exe", "xboxpcapp.exe", "gamingservices.exe",
    // anti-cheat
    "easyanticheat.exe", "easyanticheat_eos.exe", "beservice.exe", "vgc.exe", "vgtray.exe", "faceit.exe",
    // XBGG
    "xbgg.exe",
];

pub fn normalize(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

pub fn is_never_freeze(name: &str) -> bool {
    NEVER_FREEZE.contains(&normalize(name).as_str())
}

impl BoostConfig {
    pub fn should_freeze(&self, name: &str) -> bool {
        let n = normalize(name);
        self.freeze_apps && !is_never_freeze(&n) && self.freeze_list.contains(&n)
    }

    pub fn toggle_app(&mut self, name: &str) {
        let n = normalize(name);
        if is_never_freeze(&n) {
            return;
        }
        if let Some(i) = self.freeze_list.iter().position(|f| *f == n) {
            self.freeze_list.remove(i);
        } else {
            self.freeze_list.push(n);
            self.freeze_list.sort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_never_overlap_the_protected_list() {
        for name in DEFAULT_FREEZE.iter().chain(SUGGESTED) {
            assert!(!is_never_freeze(name), "{name} is protected");
            assert_eq!(*name, normalize(name), "{name} must be lower case");
        }
    }

    #[test]
    fn protected_processes_are_never_frozen() {
        let mut c = BoostConfig::default();
        c.freeze_list.push("steam.exe".into());
        c.freeze_list.push("explorer.exe".into());
        assert!(!c.should_freeze("Steam.exe"));
        assert!(!c.should_freeze("explorer.exe"));
        assert!(c.should_freeze("Chrome.EXE"));
        c.toggle_app("svchost.exe");
        assert!(!c.freeze_list.contains(&"svchost.exe".to_string()));
    }

    #[test]
    fn toggling_and_master_switch() {
        let mut c = BoostConfig::default();
        assert!(!c.should_freeze("discord.exe"));
        c.toggle_app("Discord.exe");
        assert!(c.should_freeze("discord.exe"));
        c.toggle_app("discord.exe");
        assert!(!c.should_freeze("discord.exe"));
        c.freeze_apps = false;
        assert!(!c.should_freeze("chrome.exe"));
    }

    #[test]
    fn old_config_without_boost_section_gets_defaults() {
        let c: BoostConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(c, BoostConfig::default());
        let partial: BoostConfig = serde_json::from_str(r#"{"enabled": false}"#).unwrap();
        assert!(!partial.enabled && partial.freeze_apps);
    }
}
