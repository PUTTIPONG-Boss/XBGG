//! Lazy loading of the large background ("hero") artwork.
//!
//! Only the focused entry's hero is kept in memory. It is decoded and
//! downscaled on a worker thread so scrolling never stalls the UI, and
//! stale requests (the user already moved on) are dropped.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer};

use crate::AppWindow;

/// The hero is drawn dimmed behind the UI, so full resolution is wasted memory.
const HERO_MAX_WIDTH: u32 = 1600;
/// Skip decoding while the user is scrolling quickly.
const DEBOUNCE: Duration = Duration::from_millis(90);

#[derive(Default)]
pub struct HeroLoader {
    generation: Arc<AtomicU64>,
}

impl HeroLoader {
    pub fn request(&self, ui: &AppWindow, path: Option<PathBuf>) {
        let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let Some(path) = path else {
            ui.set_has_hero(false);
            ui.set_hero(Image::default());
            return;
        };

        let latest = self.generation.clone();
        let weak = ui.as_weak();
        thread::spawn(move || {
            thread::sleep(DEBOUNCE);
            if latest.load(Ordering::SeqCst) != gen {
                return;
            }
            let decoded = decode(&path);
            let _ = weak.upgrade_in_event_loop(move |ui| {
                if latest.load(Ordering::SeqCst) != gen {
                    return;
                }
                match decoded {
                    Some(buf) => {
                        ui.set_hero(Image::from_rgba8(buf));
                        ui.set_has_hero(true);
                    }
                    None => {
                        ui.set_has_hero(false);
                        ui.set_hero(Image::default());
                    }
                }
            });
        });
    }
}

fn decode(path: &Path) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    let img = image::open(path).ok()?;
    let img = if img.width() > HERO_MAX_WIDTH { img.thumbnail(HERO_MAX_WIDTH, u32::MAX) } else { img };
    let rgba = img.to_rgba8();
    Some(SharedPixelBuffer::clone_from_slice(rgba.as_raw(), rgba.width(), rgba.height()))
}
