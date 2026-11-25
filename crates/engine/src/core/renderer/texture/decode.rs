//! Reading image files into pixels on every core at once. Loading a scene
//! reads hundreds of MB of JPEG and PNG maps; one after the other on the
//! main thread that held the editor for seconds (2026-11-21). Decoding
//! needs no GPU, so the loaders gather the files first, decode them here,
//! and upload the pixels one by one afterwards.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

use image::RgbaImage;

/// What decoding one item gave: the pixels, or why not.
pub type Decoded = Result<Image, String>;

/// An image ready for upload: pixels, or BC7 blocks out of an asset pack.
pub enum Image {
    Rgba(RgbaImage),
    Bc7(Bc7Image),
}

/// BC7 blocks of an image with all its mip levels, the largest first
/// (see docs/asset-pack.md).
pub struct Bc7Image {
    pub width: u32,
    pub height: u32,
    pub mips: Vec<Vec<u8>>,
}

impl Image {
    pub fn dimensions(&self) -> (u32, u32) {
        match self {
            Image::Rgba(image) => image.dimensions(),
            Image::Bc7(image) => (image.width, image.height),
        }
    }

    /// The pixels, for code that works on them (blocks are decoded on the
    /// CPU: only where it is needed).
    pub fn into_rgba(self) -> RgbaImage {
        match self {
            Image::Rgba(image) => image,
            Image::Bc7(image) => image.decode(),
        }
    }
}

impl From<RgbaImage> for Image {
    fn from(image: RgbaImage) -> Self {
        Image::Rgba(image)
    }
}

impl Bc7Image {
    /// The bytes of mip `level` of a `width` × `height` image.
    pub fn level_size(width: u32, height: u32, level: u32) -> usize {
        let (w, h) = ((width >> level).max(1), (height >> level).max(1));
        crate::core::gfx::bc_blocks(w) * crate::core::gfx::bc_blocks(h) * 16
    }

    /// Mip levels of a full chain for `width` × `height`.
    pub fn level_count(width: u32, height: u32) -> u32 {
        32 - width.max(height).max(1).leading_zeros()
    }

    /// Split `bytes` (every level, the largest first) into levels.
    pub fn from_levels(width: u32, height: u32, mips: u32, bytes: &[u8]) -> Result<Self, String> {
        let mut levels = Vec::with_capacity(mips as usize);
        let mut at = 0;
        for level in 0..mips {
            let size = Self::level_size(width, height, level);
            let slice = bytes.get(at..at + size).ok_or("BC7 image cut short")?;
            levels.push(slice.to_vec());
            at += size;
        }
        Ok(Self { width, height, mips: levels })
    }

    /// The largest level's pixels.
    pub fn decode(&self) -> RgbaImage {
        let (width, height) = (self.width.max(1), self.height.max(1));
        let (bw, bh) = (crate::core::gfx::bc_blocks(width), crate::core::gfx::bc_blocks(height));
        let pitch = bw * 4 * 4;
        let mut padded = vec![0u8; pitch * bh * 4];
        if let Some(blocks) = self.mips.first() {
            for by in 0..bh {
                for bx in 0..bw {
                    let at = (by * bw + bx) * 16;
                    let Some(block) = blocks.get(at..at + 16) else { continue };
                    let start = by * 4 * pitch + bx * 16;
                    bcdec_rs::bc7(block, &mut padded[start..], pitch);
                }
            }
        }
        let mut image = RgbaImage::new(width, height);
        for y in 0..height as usize {
            let row = &padded[y * pitch..y * pitch + width as usize * 4];
            image.as_mut()[y * width as usize * 4..(y + 1) * width as usize * 4].copy_from_slice(row);
        }
        image
    }
}

/// Decode `items` (by `decode`) on up to all cores; the results in the
/// order of `items`.
pub fn decode_all<T: Sync>(items: &[T], decode: impl Fn(&T) -> Decoded + Sync) -> Vec<Decoded> {
    if items.len() < 2 {
        return items.iter().map(&decode).collect();
    }
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(items.len());
    let next = AtomicUsize::new(0);
    let mut results: Vec<Option<Decoded>> = (0..items.len()).map(|_| None).collect();
    let done: Vec<Vec<(usize, Decoded)>> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut mine = Vec::new();
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(index) else { break };
                        mine.push((index, decode(item)));
                    }
                    mine
                })
            })
            .collect();
        workers.into_iter().map(|w| w.join().unwrap_or_default()).collect()
    });
    for (index, result) in done.into_iter().flatten() {
        results[index] = Some(result);
    }
    results.into_iter().map(|r| r.unwrap_or_else(|| Err("not decoded".into()))).collect()
}

/// An image file's pixels, or its blocks when the asset pack holds it.
pub fn decode_file(path: &Path) -> Decoded {
    use crate::core::assets::pack;
    if let Some(entry) = pack::entry(path) {
        if let pack::Kind::Bc7 { width, height, mips } = entry.kind {
            let bytes = pack::read(&entry)?;
            return Bc7Image::from_levels(width, height, mips, &bytes).map(Image::Bc7);
        }
    }
    let bytes = crate::core::assets::vfs::read(path)?;
    decode_bytes(&bytes)
}

/// Encoded image bytes' pixels.
pub fn decode_bytes(bytes: &[u8]) -> Decoded {
    image::load_from_memory(bytes).map(|image| Image::Rgba(image.to_rgba8())).map_err(|e| e.to_string())
}

/// Decode the files of `paths` (duplicates once) on all cores: path →
/// pixels or why not.
pub fn decode_files(paths: &[PathBuf]) -> HashMap<PathBuf, Decoded> {
    let mut unique: Vec<PathBuf> = paths.to_vec();
    unique.sort();
    unique.dedup();
    let decoded = decode_all(&unique, |path| decode_file(path));
    unique.into_iter().zip(decoded).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_keep_the_order_of_the_items() {
        let items: Vec<u32> = (1..=50).collect();
        let decoded = decode_all(&items, |n| Ok(RgbaImage::new(*n, 1).into()));
        let widths: Vec<u32> = decoded.iter().map(|d| d.as_ref().unwrap().dimensions().0).collect();
        assert_eq!(widths, items);
    }

    #[test]
    fn a_file_that_is_no_image_says_why() {
        let path = std::env::temp_dir().join(format!("ferrite-decode-{}.png", std::process::id()));
        std::fs::write(&path, b"not an image").unwrap();
        let decoded = decode_files(&[path.clone(), path.clone()]);
        assert_eq!(decoded.len(), 1);
        assert!(decoded[&path].is_err());
        let _ = std::fs::remove_file(&path);
    }
}
