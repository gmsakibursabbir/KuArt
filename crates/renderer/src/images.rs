//! Decoded-image cache: images are decoded at the resolution the screen needs
//! (power-of-two mip levels), kept under a byte budget and evicted least-recently-used.
//! Pixels never live in the document (docs/memory-strategy.md §2.2–2.3).

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use ude_core::assets::{AssetId, AssetSource, AssetStore};
use vello::peniko::{Blob, ImageAlphaType, ImageData, ImageFormat};

/// Default budget for decoded pixels (8 GB RAM tier).
pub const DEFAULT_BUDGET: usize = 128 * 1024 * 1024;

/// Pixels decoded at one mip level, premultiplied RGBA8.
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub cpu: Arc<vello_cpu::Pixmap>,
    /// GPU copy, created only when the GPU backend draws this image.
    gpu: OnceLock<ImageData>,
}

impl DecodedImage {
    pub fn gpu(&self) -> &ImageData {
        self.gpu.get_or_init(|| ImageData {
            data: Blob::new(Arc::new(self.cpu.data_as_u8_slice().to_vec())),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::AlphaPremultiplied,
            width: self.width,
            height: self.height,
        })
    }

    fn bytes(&self) -> usize {
        self.width as usize
            * self.height as usize
            * 4
            * if self.gpu.get().is_some() { 2 } else { 1 }
    }
}

struct Entry {
    image: Arc<DecodedImage>,
    last_used: u64,
}

pub struct ImageCache {
    entries: HashMap<(AssetId, u8), Entry>,
    /// Assets that failed to decode, so a broken file is not retried every frame.
    failed: HashMap<AssetId, String>,
    budget: usize,
    used: usize,
    frame: u64,
}

impl Default for ImageCache {
    fn default() -> Self {
        Self::with_budget(DEFAULT_BUDGET)
    }
}

impl ImageCache {
    pub fn with_budget(budget: usize) -> Self {
        Self {
            entries: HashMap::new(),
            failed: HashMap::new(),
            budget,
            used: 0,
            frame: 0,
        }
    }

    /// Bytes of decoded pixels currently held.
    pub fn used_bytes(&self) -> usize {
        self.used
    }

    /// Images that could not be decoded, with the reason.
    pub fn failures(&self) -> impl Iterator<Item = (&AssetId, &String)> {
        self.failed.iter()
    }

    /// Marks a new frame for LRU bookkeeping.
    pub fn next_frame(&mut self) {
        self.frame += 1;
    }

    /// The image at the smallest mip level that still covers `needed` device pixels.
    /// `src` is the image's full size. Decodes on a cache miss (synchronously).
    pub fn get(
        &mut self,
        assets: &AssetStore,
        id: AssetId,
        src: (u32, u32),
        needed: (f64, f64),
    ) -> Option<Arc<DecodedImage>> {
        if self.failed.contains_key(&id) {
            return None;
        }
        let level = mip_level(src, needed);
        if let Some(e) = self.entries.get_mut(&(id, level)) {
            e.last_used = self.frame;
            return Some(Arc::clone(&e.image));
        }
        let decoded = match decode(assets, id, level) {
            Ok(d) => Arc::new(d),
            Err(e) => {
                self.failed.insert(id, e);
                return None;
            }
        };
        self.used += decoded.bytes();
        self.entries.insert(
            (id, level),
            Entry {
                image: Arc::clone(&decoded),
                last_used: self.frame,
            },
        );
        self.evict();
        Some(decoded)
    }

    fn evict(&mut self) {
        while self.used > self.budget && self.entries.len() > 1 {
            // Never evict what this frame is drawing.
            let Some((&key, _)) = self
                .entries
                .iter()
                .filter(|(_, e)| e.last_used < self.frame)
                .min_by_key(|(_, e)| e.last_used)
            else {
                break;
            };
            if let Some(e) = self.entries.remove(&key) {
                self.used = self.used.saturating_sub(e.image.bytes());
            }
        }
    }
}

/// Largest power-of-two reduction that keeps at least `needed` pixels on each axis.
fn mip_level(src: (u32, u32), needed: (f64, f64)) -> u8 {
    let ratio = (f64::from(src.0) / needed.0.max(1.0)).min(f64::from(src.1) / needed.1.max(1.0));
    if ratio <= 1.0 {
        0
    } else {
        (ratio.log2().floor() as u8).min(12)
    }
}

fn decode(assets: &AssetStore, id: AssetId, level: u8) -> Result<DecodedImage, String> {
    let entry = assets.get(id).ok_or("unknown asset")?;
    let AssetSource::Memory { bytes, .. } = &entry.source else {
        return Err("image bytes are not loaded".into());
    };
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let (w, h) = (img.width(), img.height());
    // Each level halves the size; also keep within the u16 pixmap limit.
    let mut shift = u32::from(level);
    while (w >> shift).max(h >> shift) > u32::from(u16::MAX) {
        shift += 1;
    }
    let (tw, th) = ((w >> shift).max(1), (h >> shift).max(1));
    let rgba = if shift == 0 {
        img.into_rgba8()
    } else {
        img.resize_exact(tw, th, image::imageops::FilterType::Triangle)
            .into_rgba8()
    };
    let pixmap = vello_cpu::Pixmap::from_parts(
        rgba.into_raw(),
        tw as u16,
        th as u16,
        vello_cpu::PixelMetadata::new(ImageAlphaType::Alpha, true),
    );
    Ok(DecodedImage {
        width: tw,
        height: th,
        cpu: Arc::new(pixmap),
        gpu: OnceLock::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ude_core::assets::{AssetEntry, AssetKind, Codec, ContentHash};

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([200, 30, 30, 255]));
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    fn store(bytes: Vec<u8>, n: u8) -> (AssetStore, AssetId) {
        let mut s = AssetStore::default();
        let id = s.insert(AssetEntry {
            hash: ContentHash([n; 32]),
            kind: AssetKind::Image {
                width: 0,
                height: 0,
            },
            source: AssetSource::Memory {
                bytes: bytes.into(),
                codec: Codec::Png,
            },
        });
        (s, id)
    }

    #[test]
    fn decodes_at_needed_level_and_evicts_under_budget() {
        let (assets, id) = store(png(1600, 1200), 1);
        let mut cache = ImageCache::with_budget(1_000_000);
        // Shown at 200×150: a 1/8 level (200×150) is enough, not the 7.7 MB original.
        let img = cache
            .get(&assets, id, (1600, 1200), (200.0, 150.0))
            .unwrap();
        assert_eq!((img.width, img.height), (200, 150));
        assert_eq!(cache.used_bytes(), 200 * 150 * 4);

        // A full-size request exceeds the budget; the older, unused level is evicted.
        cache.next_frame();
        let full = cache
            .get(&assets, id, (1600, 1200), (1600.0, 1200.0))
            .unwrap();
        assert_eq!(full.width, 1600);
        assert_eq!(cache.used_bytes(), 1600 * 1200 * 4);
    }

    #[test]
    fn broken_image_fails_once() {
        let (assets, id) = store(b"not an image".to_vec(), 2);
        let mut cache = ImageCache::default();
        assert!(cache.get(&assets, id, (10, 10), (10.0, 10.0)).is_none());
        assert_eq!(cache.failures().count(), 1);
    }
}
