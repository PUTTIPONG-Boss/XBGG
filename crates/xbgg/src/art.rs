//! Artwork, always decoded at the exact on-screen pixel size so drawing it is
//! a 1:1 copy (the UI renders on the CPU).
//!
//! - Terminal layout: only the focused game's cover ([`CoverLoader`]).
//! - Console layout: every cover for the carousel ([`CoverCache`]) plus the
//!   focused game's background art with dimming baked in ([`HeroLoader`]).
//!
//! Work happens on worker threads; requests made while the user is still
//! scrolling are dropped.

use std::cell::{OnceCell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

use image::imageops::FilterType;
use image::DynamicImage;
use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer};

use crate::AppWindow;

/// Matches `T.bg` in app.slint.
const BG: [i32; 3] = [12, 12, 12];
/// How strongly the background art shows through.
const HERO_OPACITY: f32 = 0.5;
/// Vertical fade to the background colour: (position, darkening).
const HERO_FADE: [(f32, f32); 3] = [(0.0, 0.33), (0.55, 0.8), (1.0, 1.0)];
/// Skip decoding while the user is scrolling quickly.
const DEBOUNCE: Duration = Duration::from_millis(60);

pub type Buffer = SharedPixelBuffer<Rgba8Pixel>;

/// Scale-and-crop to exactly `w`x`h`. Downscaling (the normal case) uses the
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

pub fn decode_cover(path: &Path, width: u32, height: u32) -> Option<Buffer> {
    if width == 0 || height == 0 {
        return None;
    }
    let img = fill(image::open(path).ok()?, width, height).to_rgba8();
    Some(Buffer::clone_from_slice(img.as_raw(), img.width(), img.height()))
}

// ---------------------------------------------------- one cover (terminal)

struct Job {
    gen: u64,
    path: PathBuf,
    width: u32,
    height: u32,
    ui: slint::Weak<AppWindow>,
}

/// One long-lived worker decodes the focused cover; only the newest request
/// matters.
#[derive(Default)]
pub struct CoverLoader {
    generation: Arc<AtomicU64>,
    /// Last request, to skip redundant decodes (UI thread only).
    last: RefCell<Option<(PathBuf, u32, u32)>>,
    worker: OnceCell<mpsc::Sender<Job>>,
}

impl CoverLoader {
    /// Shows the cover at `path` at `width`x`height` physical pixels, or none.
    pub fn request(&self, ui: &AppWindow, path: Option<PathBuf>, width: u32, height: u32) {
        let key = path.filter(|_| width > 0 && height > 0).map(|p| (p, width, height));
        if key.is_some() && *self.last.borrow() == key {
            return;
        }
        *self.last.borrow_mut() = key.clone();

        let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        // Hide the previous game's cover right away; never show a wrong one.
        ui.set_has_cover(false);
        ui.set_cover(Image::default());
        let Some((path, width, height)) = key else { return };

        let job = Job { gen, path, width, height, ui: ui.as_weak() };
        let _ = self.worker().send(job);
    }

    /// Forgets the last request and releases the shown image.
    pub fn reset(&self, ui: &AppWindow) {
        *self.last.borrow_mut() = None;
        self.generation.fetch_add(1, Ordering::SeqCst);
        ui.set_has_cover(false);
        ui.set_cover(Image::default());
    }

    fn worker(&self) -> &mpsc::Sender<Job> {
        self.worker.get_or_init(|| {
            let (tx, rx) = mpsc::channel::<Job>();
            let latest = self.generation.clone();
            thread::Builder::new()
                .name("xbgg-covers".into())
                .spawn(move || {
                    while let Ok(mut job) = rx.recv() {
                        // Let fast scrolling settle, then take the newest request.
                        thread::sleep(DEBOUNCE);
                        while let Ok(newer) = rx.try_recv() {
                            job = newer;
                        }
                        if latest.load(Ordering::SeqCst) != job.gen {
                            continue;
                        }
                        let Some(buf) = decode_cover(&job.path, job.width, job.height) else { continue };
                        let (gen, latest) = (job.gen, latest.clone());
                        let _ = job.ui.upgrade_in_event_loop(move |ui| {
                            if latest.load(Ordering::SeqCst) == gen {
                                ui.set_cover(Image::from_rgba8(buf));
                                ui.set_has_cover(true);
                            }
                        });
                    }
                })
                .expect("failed to spawn cover thread");
            tx
        })
    }
}

// --------------------------------------------------- all covers (console)

/// Every cover for the carousel, for one tile size. Cleared on size change.
#[derive(Default)]
pub struct CoverCache {
    size: Option<(u32, u32)>,
    images: HashMap<PathBuf, Option<Image>>,
}

impl CoverCache {
    pub fn get(&self, path: &Path) -> Option<Image> {
        self.images.get(path).cloned().flatten()
    }

    /// Decodes every cover not yet cached, in parallel, and drops covers of
    /// entries that left the library.
    pub fn ensure(&mut self, paths: &[PathBuf], width: u32, height: u32) {
        if self.size != Some((width, height)) {
            self.images.clear();
            self.size = Some((width, height));
        }
        self.images.retain(|p, _| paths.contains(p));
        let missing: Vec<&PathBuf> = paths.iter().filter(|p| !self.images.contains_key(*p)).collect();
        if missing.is_empty() {
            return;
        }
        let workers = thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(8);
        let chunk = missing.len().div_ceil(workers).max(1);
        let decoded: Vec<(&PathBuf, Option<Buffer>)> = thread::scope(|s| {
            let handles: Vec<_> = missing
                .chunks(chunk)
                .map(|part| s.spawn(move || part.iter().map(|p| (*p, decode_cover(p, width, height))).collect::<Vec<_>>()))
                .collect();
            handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
        });
        for (path, buf) in decoded {
            self.images.insert(path.clone(), buf.map(Image::from_rgba8));
        }
    }

    /// Frees everything (switching to the terminal layout).
    pub fn clear(&mut self) {
        self.images.clear();
        self.size = None;
    }
}

// ------------------------------------------------ background art (console)

#[derive(Default)]
pub struct HeroLoader {
    generation: Arc<AtomicU64>,
    last: RefCell<Option<(PathBuf, u32, u32)>>,
}

impl HeroLoader {
    /// Shows the background for `path` at `width`x`height` physical pixels.
    pub fn request(&self, ui: &AppWindow, path: Option<PathBuf>, width: u32, height: u32) {
        let key = path.filter(|_| width > 0 && height > 0).map(|p| (p, width, height));
        if key.is_some() && *self.last.borrow() == key {
            return;
        }
        *self.last.borrow_mut() = key.clone();

        let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let Some((path, width, height)) = key else {
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

    /// Forgets the last request and releases the shown image.
    pub fn reset(&self, ui: &AppWindow) {
        *self.last.borrow_mut() = None;
        self.generation.fetch_add(1, Ordering::SeqCst);
        ui.set_has_hero(false);
        ui.set_hero(Image::default());
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

/// Builds the background in one fixed-point pass: cover-fit crop with
/// bilinear sampling, 50% dimming and a fade to the background colour.
pub fn bake_hero(path: &Path, width: u32, height: u32) -> Option<Buffer> {
    if width == 0 || height == 0 {
        return None;
    }
    let mut src = image::open(path).ok()?;
    if src.width() > width * 2 && src.height() > height * 2 {
        src = src.thumbnail(width * 2, height * 2);
    }
    let src = src.to_rgb8();
    let (sw, sh) = (src.width() as usize, src.height() as usize);
    let (w, h) = (width as usize, height as usize);
    let scale = (w as f32 / sw as f32).max(h as f32 / sh as f32);
    let off_x = (sw as f32 - w as f32 / scale) / 2.0;
    let off_y = (sh as f32 - h as f32 / scale) / 2.0;

    // Weights in 0..=256; per-column taps shared by every row.
    let cols: Vec<(usize, usize, u32)> = (0..w)
        .map(|x| {
            let sx = (off_x + (x as f32 + 0.5) / scale - 0.5).clamp(0.0, (sw - 1) as f32);
            let x0 = sx.floor() as usize;
            (x0 * 3, (x0 + 1).min(sw - 1) * 3, ((sx - x0 as f32) * 256.0) as u32)
        })
        .collect();

    let raw = src.as_raw();
    let mut vrow = vec![0u32; sw * 3];
    let mut buf = Buffer::new(width, height);
    for (y, row) in buf.make_mut_slice().chunks_exact_mut(w).enumerate() {
        let sy = (off_y + (y as f32 + 0.5) / scale - 0.5).clamp(0.0, (sh - 1) as f32);
        let y0 = sy.floor() as usize;
        let y1 = (y0 + 1).min(sh - 1);
        let fy = ((sy - y0 as f32) * 256.0) as u32;
        let top = &raw[y0 * sw * 3..(y0 + 1) * sw * 3];
        let bot = &raw[y1 * sw * 3..(y1 + 1) * sw * 3];
        for ((v, &t), &b) in vrow.iter_mut().zip(top).zip(bot) {
            *v = t as u32 * (256 - fy) + b as u32 * fy;
        }
        // result = BG + (src - BG) * opacity * (1 - fade)
        let k = (HERO_OPACITY * (1.0 - fade_at(y as f32 / (h.max(2) - 1) as f32)) * 256.0) as i32;
        for (px, &(x0, x1, fx)) in row.iter_mut().zip(&cols) {
            let mut c = [0u8; 3];
            for (i, out) in c.iter_mut().enumerate() {
                let s = (vrow[x0 + i] * (256 - fx) + vrow[x1 + i] * fx) >> 16;
                *out = (BG[i] + (((s as i32 - BG[i]) * k) >> 8)).clamp(0, 255) as u8;
            }
            *px = Rgba8Pixel { r: c[0], g: c[1], b: c[2], a: 255 };
        }
    }
    Some(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbaImage;

    fn sample(dir: &Path) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join("art.png");
        // Wrong aspect ratio on purpose: must be cropped, not stretched.
        RgbaImage::from_pixel(300, 200, image::Rgba([200, 100, 50, 255])).save(&path).unwrap();
        path
    }

    #[test]
    fn cover_is_decoded_at_exact_size() {
        let dir = std::env::temp_dir().join(format!("xbgg-art-{}", std::process::id()));
        let path = sample(&dir);
        for (w, h) in [(40, 60), (400, 600)] {
            let buf = decode_cover(&path, w, h).unwrap();
            assert_eq!((buf.width(), buf.height()), (w, h));
            assert!(buf.as_slice().iter().all(|p| p.a == 255), "square corners: fully opaque");
        }
        assert!(decode_cover(&path, 0, 10).is_none());
        assert!(decode_cover(&dir.join("missing.png"), 10, 10).is_none());

        let mut cache = CoverCache::default();
        cache.ensure(std::slice::from_ref(&path), 30, 45);
        assert!(cache.get(&path).is_some());
        cache.ensure(&[], 30, 45);
        assert!(cache.get(&path).is_none(), "entries that left the library are dropped");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hero_is_baked_opaque_and_fades_to_background() {
        let dir = std::env::temp_dir().join(format!("xbgg-hero-{}", std::process::id()));
        let path = sample(&dir);
        let buf = bake_hero(&path, 160, 90).unwrap();
        assert_eq!((buf.width(), buf.height()), (160, 90));
        assert!(buf.as_slice().iter().all(|p| p.a == 255));
        let last = buf.as_slice()[(89 * 160) as usize];
        assert_eq!((last.r as i32, last.g as i32, last.b as i32), (BG[0], BG[1], BG[2]));
        assert!((fade_at(0.0) - 0.33).abs() < 1e-6 && (fade_at(1.0) - 1.0).abs() < 1e-6);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
