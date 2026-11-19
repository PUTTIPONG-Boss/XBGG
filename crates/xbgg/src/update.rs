//! In-app updates from GitHub Releases.
//!
//! Flow: query the latest release → download `xbgg.exe` and `xbgg.exe.sha256`
//! → verify the SHA-256 → swap the running executable (Windows allows renaming
//! a running .exe) → restart. Nothing is installed silently: the user confirms
//! every update, and a failed swap rolls back to the current executable.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

pub const REPO: &str = "PUTTIPONG-Boss/XBGG";
const EXE_ASSET: &str = "xbgg.exe";
const SHA_ASSET: &str = "xbgg.exe.sha256";
const MAX_EXE_BYTES: u64 = 64 * 1024 * 1024;
/// Lets testers exercise the update path against a real release.
const PRETEND_VERSION_ENV: &str = "XBGG_PRETEND_VERSION";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u64, pub u64, pub u64);

impl Version {
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().trim_start_matches(['v', 'V']);
        // Ignore pre-release / build suffixes ("1.2.3-beta", "1.2.3+abc").
        let core = s.split(['-', '+']).next()?;
        let mut parts = core.split('.').map(|p| p.parse::<u64>());
        let major = parts.next()?.ok()?;
        let minor = parts.next().unwrap_or(Ok(0)).ok()?;
        let patch = parts.next().unwrap_or(Ok(0)).ok()?;
        if parts.next().is_some() {
            return None;
        }
        Some(Version(major, minor, patch))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

pub fn current_version() -> Version {
    std::env::var(PRETEND_VERSION_ENV)
        .ok()
        .and_then(|v| Version::parse(&v))
        .unwrap_or_else(|| Version::parse(env!("CARGO_PKG_VERSION")).expect("valid crate version"))
}

#[derive(Debug, Clone)]
pub struct Release {
    pub version: Version,
    pub exe_url: String,
    pub sha_url: String,
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .tls_config(
            TlsConfig::builder()
                .provider(TlsProvider::NativeTls)
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .timeout_global(Some(timeout))
        .user_agent(concat!("XBGG/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent()
}

/// Returns the latest release if it is newer than the running version.
pub fn check() -> Result<Option<Release>> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let mut resp = match agent(Duration::from_secs(15))
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .call()
    {
        Ok(r) => r,
        // No published release yet.
        Err(ureq::Error::StatusCode(404)) => return Ok(None),
        Err(e) => return Err(e).context("contacting GitHub"),
    };
    let rel: GhRelease = resp.body_mut().read_json().context("reading release info")?;
    pick_update(rel, current_version())
}

fn pick_update(rel: GhRelease, current: Version) -> Result<Option<Release>> {
    if rel.draft || rel.prerelease {
        return Ok(None);
    }
    let version = Version::parse(&rel.tag_name).ok_or_else(|| anyhow!("unrecognised tag {}", rel.tag_name))?;
    if version <= current {
        return Ok(None);
    }
    let find = |name: &str| rel.assets.iter().find(|a| a.name == name).map(|a| a.browser_download_url.clone());
    let (Some(exe_url), Some(sha_url)) = (find(EXE_ASSET), find(SHA_ASSET)) else {
        bail!("release {} is missing {EXE_ASSET} or {SHA_ASSET}", rel.tag_name);
    };
    Ok(Some(Release { version, exe_url, sha_url }))
}

/// Downloads, verifies and swaps in the new executable. Returns the path to
/// start; the caller restarts into it and exits.
pub fn install(release: &Release) -> Result<PathBuf> {
    let exe = std::env::current_exe().context("locating xbgg.exe")?;
    let http = agent(Duration::from_secs(300));

    let sha_text = http
        .get(&release.sha_url)
        .call()
        .context("downloading checksum")?
        .body_mut()
        .with_config()
        .limit(4096)
        .read_to_string()
        .context("reading checksum")?;
    let expected = parse_sha256(&sha_text).ok_or_else(|| anyhow!("checksum file is malformed"))?;

    let bytes = http
        .get(&release.exe_url)
        .call()
        .context("downloading update")?
        .body_mut()
        .with_config()
        .limit(MAX_EXE_BYTES)
        .read_to_vec()
        .context("reading update")?;

    let actual = hex(&Sha256::digest(&bytes));
    if actual != expected {
        bail!("checksum mismatch (expected {expected}, got {actual}); update discarded");
    }
    if !bytes.starts_with(b"MZ") {
        bail!("downloaded file is not a Windows executable");
    }

    swap_executable(&exe, &bytes)?;
    Ok(exe)
}

fn swap_executable(exe: &Path, bytes: &[u8]) -> Result<()> {
    let new = exe.with_extension("exe.new");
    let old = exe.with_extension("exe.old");
    fs::write(&new, bytes).with_context(|| {
        format!("writing {} (is the folder writable?)", new.display())
    })?;

    let _ = fs::remove_file(&old);
    if let Err(e) = fs::rename(exe, &old) {
        let _ = fs::remove_file(&new);
        return Err(e).context("moving the current version aside");
    }
    if let Err(e) = fs::rename(&new, exe) {
        // Roll back so XBGG keeps working.
        let _ = fs::rename(&old, exe);
        let _ = fs::remove_file(&new);
        return Err(e).context("putting the new version in place");
    }
    Ok(())
}

/// Removes the previous executable left behind by an update.
pub fn cleanup_previous() {
    let Ok(exe) = std::env::current_exe() else { return };
    let old = exe.with_extension("exe.old");
    if !old.exists() {
        return;
    }
    std::thread::spawn(move || {
        // The old process may still be shutting down.
        for _ in 0..20 {
            if fs::remove_file(&old).is_ok() || !old.exists() {
                return;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    });
}

/// Accepts `sha256sum` style ("<hex>  xbgg.exe") or a bare hex digest.
fn parse_sha256(text: &str) -> Option<String> {
    let token = text.split_whitespace().next()?.to_ascii_lowercase();
    (token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit())).then_some(token)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, assets: &[&str]) -> GhRelease {
        GhRelease {
            tag_name: tag.into(),
            draft: false,
            prerelease: false,
            assets: assets
                .iter()
                .map(|n| GhAsset { name: n.to_string(), browser_download_url: format!("https://x/{n}") })
                .collect(),
        }
    }

    #[test]
    fn parses_versions() {
        assert_eq!(Version::parse("v0.1.0"), Some(Version(0, 1, 0)));
        assert_eq!(Version::parse("1.2"), Some(Version(1, 2, 0)));
        assert_eq!(Version::parse("v2.0.1-beta.1"), Some(Version(2, 0, 1)));
        assert_eq!(Version::parse("1.2.3.4"), None);
        assert_eq!(Version::parse("latest"), None);
        assert!(Version(0, 10, 0) > Version(0, 9, 9));
    }

    #[test]
    fn picks_only_newer_complete_releases() {
        let cur = Version(0, 1, 0);
        assert!(pick_update(release("v0.1.0", &[EXE_ASSET, SHA_ASSET]), cur).unwrap().is_none());
        assert!(pick_update(release("v0.0.9", &[EXE_ASSET, SHA_ASSET]), cur).unwrap().is_none());
        let up = pick_update(release("v0.2.0", &[EXE_ASSET, SHA_ASSET]), cur).unwrap().unwrap();
        assert_eq!(up.version, Version(0, 2, 0));
        assert!(pick_update(release("v0.2.0", &[EXE_ASSET]), cur).is_err());

        let mut pre = release("v0.3.0", &[EXE_ASSET, SHA_ASSET]);
        pre.prerelease = true;
        assert!(pick_update(pre, cur).unwrap().is_none());
    }

    #[test]
    fn parses_checksum_files() {
        let h = "a".repeat(64);
        assert_eq!(parse_sha256(&format!("{h}  xbgg.exe\n")), Some(h.clone()));
        assert_eq!(parse_sha256(&h.to_uppercase()), Some(h));
        assert_eq!(parse_sha256("nothex"), None);
    }

    #[test]
    fn swap_replaces_and_keeps_backup() {
        let dir = std::env::temp_dir().join(format!("xbgg-swap-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("xbgg.exe");
        fs::write(&exe, b"MZ old").unwrap();
        swap_executable(&exe, b"MZ new").unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"MZ new");
        assert_eq!(fs::read(exe.with_extension("exe.old")).unwrap(), b"MZ old");
        assert!(!exe.with_extension("exe.new").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
