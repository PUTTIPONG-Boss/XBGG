pub mod epic;
pub mod gog;
pub mod steam;

use crate::model::Game;

/// Runs every store scanner. A failing scanner is reported as a warning and
/// does not prevent the others from contributing games.
pub fn scan_all() -> (Vec<Game>, Vec<String>) {
    let scanners: [(&str, fn() -> anyhow::Result<Vec<Game>>); 3] =
        [("Steam", steam::scan), ("Epic Games", epic::scan), ("GOG", gog::scan)];

    let mut games = Vec::new();
    let mut warnings = Vec::new();
    for (name, scan) in scanners {
        match scan() {
            Ok(found) => games.extend(found),
            Err(e) => warnings.push(format!("{name}: {e:#}")),
        }
    }
    (games, warnings)
}
