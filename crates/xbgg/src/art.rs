//! Artwork preparation for the software renderer.
//!
//! Everything is decoded once at the exact on-screen pixel size so drawing a
//! frame is a plain 1:1 copy:
//! - covers are resized to the tile size with rounded corners baked in;
//! - the focused game's background ("hero") is resized to the window and has
//!   the dimming and fade-to-black gradient baked in, then shown as one opaque
//!   image. It is decoded on a worker thread, and stale requests are dropped.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use image::imageops::FilterType;
use image::{DynamicImage, RgbaImage};
use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer};

use crate::AppWindow;

/// Matches `Palette.bg` in app.slint.
const BG: [f32; 3] = [7.0, 9.0, 13.0];
/// How strongly the hero shows through.
const HERO_OPACITY: f32 = 0.5;
/// Vertical fade to the background colour: (position, darkening).
const HERO_FADE: [(f32, f32); 3] = [(0.0, 0.33), (0.55, 0.8), (1.0, 1.0)];
/// Skip decoding while the user is scrolling quickly.
const DEBOUNCE: Duration = Duration::from_millis(90);

pub type Buffer = SharedPixelBuffer<Rgba8Pixel>;

fn to_buffer(img: &RgbaImage) -> Buffer {
    Buffer::clone_from_slice(img.as_raw(), img.width(), img.height())
}

fn open(path: &Path) -> Option<DynamicImage> {
    image::open(path).ok()
}

// ---------------------------------------------------------------- covers

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CoverSize {
    pub width: u32,
    pub height: u32,
    pub radius: u32,
}

/// Decoded covers for one tile size. Cleared when the size changes.
#[derive(Default)]
pub struct CoverCache {
    size: Option<CoverSize>,
    images: HashMap<PathBuf, Option<Image>>,
}

impl CoverCache {
    pub fn get(&self, path: &Path) -> Option<Image> {
        self.images.get(path).cloned().flatten()
    }

    /// Decodes every cover not yet cached, in parallel.
    pub fn ensure(&mut self, paths: &[PathBuf], size: CoverSize) {
        if self.size != Some(size) {
            self.images.clear();
            self.size = Some(size);
        }
        let missing: Vec<&PathBuf> = paths.iter().filter(|p| !self.images.contains_key(*p)).collect();
        if missing.is_empty() {
            return;
        }
        for (path, buf) in decode_parallel(&missing, size) {
            self.images.insert(path.clone(), buf.map(Image::from_rgba8));
        }
    }

    /// Drops covers of entries that left the library.
    pub fn retain(&mut self, paths: &[PathBuf]) {
        self.images.retain(|p, _| paths.contains(p));
    }
}

fn decode_parallel<'a>(paths: &[&'a PathBuf], size: CoverSize) -> Vec<(&'a PathBuf, Option<Buffer>)> {
    let workers = thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(8);
    let chunk = paths.len().div_ceil(workers).max(1);
    thread::scope(|s| {
        let handles: Vec<_> = paths
            .chunks(chunk)
            .map(|part| s.spawn(move || part.iter().map(|p| (*p, decode_cover(p, size))).collect::<Vec<_>>()))
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    })
}

/// Scale-and-crop to exactly `w`×`h`. Downscaling (the normal case) uses the
/// fast area-averaging `thumbnail`; the generic filtered resize is ~20x slower.
fn fill(img: DynamicImage, w: u32, h: u32) -> DynamicImage {
    let (sw, sh) = (img.width().max(1), img.height().max(1));
    let scale = (w as f32 / sw as f32).max(h as f32 / sh as f32);
    let nw = ((sw as f32 * scale).round() as u32).max(w);
    let nh = ((sh as f32 * scale).round() as u32).max(h);
    let scaled = if scale < 1.0 {
        img.thumbnail_exact(nw, nh)
    } else {
        img.resize_exact(nw, nh, FilterType::Triangle)
    };
    scaled.crop_imm((nw - w) / 2, (nh - h) / 2, w, h)
}

pub fn decode_cover(path: &Path, size: CoverSize) -> Option<Buffer> {
    if size.width == 0 || size.height == 0 {
        return None;
    }
    let img = fill(open(path)?, size.width, size.height);
    let mut rgba = img.to_rgba8();
    round_corners(&mut rgba, size.radius);
    Some(to_buffer(&rgba))
}

/// Makes the corners transparent with a one-pixel anti-aliased edge.
fn round_corners(img: &mut RgbaImage, radius: u32) {
    let (w, h) = img.dimensions();
    let r = radius.min(w / 2).min(h / 2);
    if r == 0 {
        return;
    }
    let rf = r as f32;
    for cy in 0..r {
        for cx in 0..r {
            // Distance from the pixel centre to the corner circle's centre.
            let dx = rf - (cx as f32 + 0.5);
            let dy = rf - (cy as f32 + 0.5);
            let coverage = (rf - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0);
            if coverage >= 1.0 {
                continue;
            }
            for (x, y) in [(cx, cy), (w - 1 - cx, cy), (cx, h - 1 - cy), (w - 1 - cx, h - 1 - cy)] {
                let p = img.get_pixel_mut(x, y);
                p.0[3] = (p.0[3] as f32 * coverage).round() as u8;
            }
        }
    }
}

// ------------------------------------------------------------------ hero

#[derive(Default)]
pub struct HeroLoader {
    generation: Arc<AtomicU64>,
    /// Last request, to skip redundant re-decodes (UI thread only).
    last: std::cell::RefCell<Option<(PathBuf, u32, u32)>>,
}

impl HeroLoader {
    /// Shows the background for `path` at `width`×`height` physical pixels.
    pub fn request(&self, ui: &AppWindow, path: Option<PathBuf>, width: u32, height: u32) {
        let key = path.clone().filter(|_| width > 0 && height > 0).map(|p| (p, width, height));
        if key.is_some() && *self.last.borrow() == key {
            return;
        }
        *self.last.borrow_mut() = key;

        let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let Some(path) = path.filter(|_| width > 0 && height > 0) else {
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
            let baked = bake_hero(&path, width, height);
            if latest.load(Ordering::SeqCst) != gen {
                return;
            }
            let _ = weak.upgrade_in_event_loop(move |ui| {
                if latest.load(Ordering::SeqCst) != gen {
                    return;
                }
                match baked {
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

fn fade_at(t: f32) -> f32 {
    let mut prev = HERO_FADE[0];
    for stop in HERO_FADE {
        if t <= stop.0 {
            let span = (stop.0 - prev.0).max(f32::EPSILON);
            return prev.1 + (stop.1 - prev.1) * ((t - prev.0) / span).clamp(0.0, 1.0);
        }
        prev = stop;
    }
    prev.1
}

/// Builds the background in a single pass: cover-fit crop with bilinear
/// sampling, 50% dimming and the vertical fade, straight into the output
/// buffer. The art is heavily dimmed, so bilinear quality is plenty.
pub fn bake_hero(path: &Path, width: u32, height: u32) -> Option<Buffer> {
    if width == 0 || height == 0 {
        return None;
    }
    let mut src = open(path)?;
    // Very large art: shrink cheaply first so bilinear sampling doesn't alias.
    if src.width() > width * 2 && src.height() > height * 2 {
        src = src.thumbnail(width * 2, height * 2);
    }
    let src = src.to_rgb8();
    let (sw, sh) = (src.width() as usize, src.height() as usize);
    let (w, h) = (width as usize, height as usize);
    let scale = (w as f32 / sw as f32).max(h as f32 / sh as f32);
    let off_x = (sw as f32 - w as f32 / scale) / 2.0;
    let off_y = (sh as f32 - h as f32 / scale) / 2.0;

    // Fixed point: weights in 0..=256 (8 fractional bits).
    // Per-column source taps, shared by every row.
    let cols: Vec<(usize, usize, u32)> = (0..w)
        .map(|x| {
            let sx = (off_x + (x as f32 + 0.5) / scale - 0.5).clamp(0.0, (sw - 1) as f32);
            let x0 = sx.floor() as usize;
            (x0 * 3, (x0 + 1).min(sw - 1) * 3, ((sx - x0 as f32) * 256.0) as u32)
        })
        .collect();

    let raw = src.as_raw();
    let bg = BG.map(|c| c as i32);
    // One source row blended vertically (values scaled by 256).
    let mut vrow = vec![0u32; sw * 3];
    let mut buf = Buffer::new(width, height);
    let pixels = buf.make_mut_slice();
    for (y, row) in pixels.chunks_exact_mut(w).enumerate() {
        let sy = (off_y + (y as f32 + 0.5) / scale - 0.5).clamp(0.0, (sh - 1) as f32);
        let y0 = sy.floor() as usize;
        let y1 = (y0 + 1).min(sh - 1);
        let fy = ((sy - y0 as f32) * 256.0) as u32;
        let top = &raw[y0 * sw * 3..(y0 + 1) * sw * 3];
        let bot = &raw[y1 * sw * 3..(y1 + 1) * sw * 3];
        for ((v, &t), &b) in vrow.iter_mut().zip(top).zip(bot) {
            *v = t as u32 * (256 - fy) + b as u32 * fy;
        }

        // result = BG + (src - BG) * k, where k folds the 50% opacity and the
        // fade towards the background: k = opacity * (1 - dark).
        let k = HERO_OPACITY * (1.0 - fade_at(y as f32 / (h.max(2) - 1) as f32));
        let k = (k * 256.0) as i32;
        for (px, &(x0, x1, fx)) in row.iter_mut().zip(&cols) {
            let mut c = [0u8; 3];
            for (i, out) in c.iter_mut().enumerate() {
                // vrow is ×256, horizontal weight ×256 → ×65536.
                let s = (vrow[x0 + i] * (256 - fx) + vrow[x1 + i] * fx) >> 16;
                *out = (bg[i] + (((s as i32 - bg[i]) * k) >> 8)).clamp(0, 255) as u8;
            }
            *px = Rgba8Pixel { r: c[0], g: c[1], b: c[2], a: 255 };
        }
    }
    Some(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corners_become_transparent_but_edges_stay() {
        let mut img = RgbaImage::from_pixel(40, 60, image::Rgba([200, 100, 50, 255]));
        round_corners(&mut img, 10);
        assert_eq!(img.get_pixel(0, 0).0[3], 0);
        assert_eq!(img.get_pixel(39, 59).0[3], 0);
        assert_eq!(img.get_pixel(20, 0).0[3], 255, "top edge centre untouched");
        assert_eq!(img.get_pixel(20, 30).0[3], 255);
    }

    #[test]
    fn fade_interpolates_between_stops() {
        assert!((fade_at(0.0) - 0.33).abs() < 1e-6);
        assert!((fade_at(1.0) - 1.0).abs() < 1e-6);
        let mid = fade_at(0.275);
        assert!(mid > 0.33 && mid < 0.8);
    }

    /// `XBGG_BENCH_HERO=<jpg> cargo test --release -p xbgg bench_hero -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_hero() {
        let Some(path) = std::env::var_os("XBGG_BENCH_HERO") else { return };
        let path = PathBuf::from(path);
        for _ in 0..3 {
            let t = std::time::Instant::now();
            let src = open(&path).unwrap();
            let decoded = t.elapsed();
            drop(src);
            let t = std::time::Instant::now();
            bake_hero(&path, 1920, 1080).unwrap();
            println!("decode {decoded:?}  full bake {:?}", t.elapsed());
        }
    }

    #[test]
    fn baked_hero_is_opaque_and_exact_size() {
        let dir = std::env::temp_dir().join(format!("xbgg-art-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hero.png");
        RgbaImage::from_pixel(300, 100, image::Rgba([255, 255, 255, 255])).save(&path).unwrap();
        let buf = bake_hero(&path, 160, 90).unwrap();
        assert_eq!((buf.width(), buf.height()), (160, 90));
        assert!(buf.as_slice().iter().all(|p| p.a == 255));
        // Bottom row fades fully to the background colour.
        let last = buf.as_slice()[(89 * 160) as usize];
        assert_eq!((last.r, last.g, last.b), (7, 9, 13));
        let cover = decode_cover(&path, CoverSize { width: 40, height: 60, radius: 8 }).unwrap();
        assert_eq!((cover.width(), cover.height()), (40, 60));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
