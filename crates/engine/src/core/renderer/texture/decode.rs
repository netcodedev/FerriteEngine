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
pub type Decoded = Result<RgbaImage, String>;

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

/// An image file's pixels.
pub fn decode_file(path: &Path) -> Decoded {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    decode_bytes(&bytes)
}

/// Encoded image bytes' pixels.
pub fn decode_bytes(bytes: &[u8]) -> Decoded {
    image::load_from_memory(bytes).map(|image| image.to_rgba8()).map_err(|e| e.to_string())
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
        let decoded = decode_all(&items, |n| Ok(RgbaImage::new(*n, 1)));
        let widths: Vec<u32> = decoded.iter().map(|d| d.as_ref().unwrap().width()).collect();
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
