//! Cover art for the focused entry.
//!
//! Only one cover is kept in memory: the focused game's, decoded on a worker
//! thread at exactly the on-screen pixel size so drawing it is a 1:1 copy.
//! Requests made while the user is still scrolling are dropped.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

use image::imageops::FilterType;
use image::DynamicImage;
use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer};

use crate::AppWindow;

/// Skip decoding while the user is scrolling quickly.
const DEBOUNCE: Duration = Duration::from_millis(60);

pub type Buffer = SharedPixelBuffer<Rgba8Pixel>;

struct Job {
    gen: u64,
    path: PathBuf,
    width: u32,
    height: u32,
    ui: slint::Weak<AppWindow>,
}

/// One long-lived worker decodes covers; only the newest request matters.
#[derive(Default)]
pub struct CoverLoader {
    generation: Arc<AtomicU64>,
    /// Last request, to skip redundant decodes (UI thread only).
    last: std::cell::RefCell<Option<(PathBuf, u32, u32)>>,
    worker: std::cell::OnceCell<mpsc::Sender<Job>>,
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

    /// Forgets the last request (e.g. after the library changed).
    pub fn reset(&self) {
        *self.last.borrow_mut() = None;
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbaImage;

    #[test]
    fn cover_is_decoded_at_exact_size() {
        let dir = std::env::temp_dir().join(format!("xbgg-art-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cover.png");
        // Wrong aspect ratio on purpose: must be cropped, not stretched.
        RgbaImage::from_pixel(300, 200, image::Rgba([200, 100, 50, 255])).save(&path).unwrap();
        for (w, h) in [(40, 60), (400, 600)] {
            let buf = decode_cover(&path, w, h).unwrap();
            assert_eq!((buf.width(), buf.height()), (w, h));
            assert!(buf.as_slice().iter().all(|p| p.a == 255));
        }
        assert!(decode_cover(&path, 0, 10).is_none());
        assert!(decode_cover(&dir.join("missing.png"), 10, 10).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
