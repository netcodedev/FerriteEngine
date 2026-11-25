//! Cooking a project's assets into an asset pack for an export (see
//! docs/asset-pack.md): models into cooked scenes, images into BC7 blocks
//! with their mips. What is cooked is kept in `.ferrite-cache/cook/`, so
//! the next export only cooks what changed.

use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
};

use image::RgbaImage;

use super::pack::{Kind, PackWriter};

/// Images in these folders stay loose files: the terrain combines their
/// channels on the CPU, which a lossy copy would make worse.
pub const LOOSE_IMAGE_FOLDERS: [&str; 1] = ["assets/textures/terrain"];

/// Cooked files of an older layout are not reused.
const CACHE_VERSION: u32 = 1;

/// What cooking a project gave.
#[derive(Debug, Default)]
pub struct Cooked {
    /// Files in the pack, project relative.
    pub packed: Vec<String>,
    /// Files to copy loose, project relative.
    pub loose: Vec<String>,
    /// Files that could not be cooked (copied loose instead), and why.
    pub failed: Vec<(String, String)>,
    /// Cooked anew (the rest came from the cache).
    pub cooked: usize,
    /// Size of the pack written.
    pub pack_size: u64,
}

enum Plan {
    Model,
    Image,
    Loose,
}

fn plan(relative: &str) -> Plan {
    let extension = Path::new(relative)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase)
        .unwrap_or_default();
    if super::import::MODEL_EXTENSIONS.contains(&extension.as_str()) {
        Plan::Model
    } else if crate::core::model::material_maps::IMAGE_EXTENSIONS.contains(&extension.as_str())
        && !LOOSE_IMAGE_FOLDERS.iter().any(|folder| relative.starts_with(&format!("{folder}/")))
    {
        Plan::Image
    } else {
        Plan::Loose
    }
}

/// Cook every file below `folders` of the project at `project_dir` (the
/// working directory) into the pack at `pack_path`. `progress` hears
/// "what, done, of".
pub fn cook_project(
    project_dir: &Path,
    folders: &[&str],
    pack_path: &Path,
    progress: &(dyn Fn(&str, usize, usize) + Sync),
) -> Result<Cooked, String> {
    let mut files = Vec::new();
    for folder in folders {
        collect(&project_dir.join(folder), project_dir, &mut files);
    }
    files.sort();
    let cache_dir = project_dir.join(".ferrite-cache").join("cook");
    std::fs::create_dir_all(&cache_dir).map_err(|e| format!("{}: {e}", cache_dir.display()))?;

    let mut cooked = Cooked::default();
    let mut work = Vec::new();
    for relative in files {
        match plan(&relative) {
            Plan::Loose => cooked.loose.push(relative),
            Plan::Model | Plan::Image => work.push(relative),
        }
    }

    // Cook on every core; Assimp and the encoder both run per thread.
    let total = work.len();
    let done = AtomicUsize::new(0);
    let fresh = AtomicUsize::new(0);
    let results: Mutex<Vec<(String, Result<(Kind, PathBuf), String>)>> = Mutex::new(Vec::new());
    let next = AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(total.max(1));
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(relative) = work.get(index) else { break };
                let result = cook_cached(project_dir, &cache_dir, relative, &fresh);
                results.lock().unwrap_or_else(|e| e.into_inner()).push((relative.clone(), result));
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                progress(relative, n, total);
            });
        }
    });
    let mut results = results.into_inner().unwrap_or_else(|e| e.into_inner());
    results.sort_by(|a, b| a.0.cmp(&b.0));

    let mut writer = PackWriter::create(pack_path)?;
    for (relative, result) in results {
        match result {
            Ok((kind, cached)) => {
                let bytes = std::fs::read(&cached).map_err(|e| format!("{}: {e}", cached.display()))?;
                writer.add(&relative, kind, &bytes[HEADER..])?;
                cooked.packed.push(relative);
            }
            Err(e) => {
                cooked.failed.push((relative.clone(), e));
                cooked.loose.push(relative);
            }
        }
    }
    // A model's companions (an OBJ's .mtl, a glTF's .bin) are only needed
    // loose when the model itself is: keep them then.
    let failed_folders: Vec<String> = cooked
        .failed
        .iter()
        .filter(|(path, _)| matches!(plan(path), Plan::Model))
        .filter_map(|(path, _)| path.rsplit_once('/').map(|(folder, _)| folder.to_string()))
        .collect();
    cooked.loose.retain(|relative| {
        let extension = relative.rsplit('.').next().unwrap_or("").to_lowercase();
        let companion = matches!(extension.as_str(), "mtl" | "bin" | "blend" | "blend1");
        let folder = relative.rsplit_once('/').map_or("", |(folder, _)| folder);
        !companion || failed_folders.iter().any(|f| f == folder)
    });
    cooked.loose.sort();
    cooked.cooked = fresh.load(Ordering::Relaxed);
    cooked.pack_size = writer.finish()?;
    Ok(cooked)
}

fn collect(dir: &Path, root: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, root, out);
        } else if let Ok(relative) = path.strip_prefix(root) {
            let parts: Vec<String> =
                relative.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
            out.push(parts.join("/"));
        }
    }
}

/// Bytes before a cached file's data: its kind.
const HEADER: usize = 24;

/// The cooked file of `relative` in the cache, cooked now if it is not
/// there for this source (size, time, cook version).
fn cook_cached(project_dir: &Path, cache_dir: &Path, relative: &str, fresh: &AtomicUsize) -> Result<(Kind, PathBuf), String> {
    let source = project_dir.join(relative);
    let meta = std::fs::metadata(&source).map_err(|e| format!("{relative}: {e}"))?;
    let modified = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
    let key = fnv(&format!("{CACHE_VERSION}|{}|{relative}|{}|{modified}", super::cooked_scene::VERSION, meta.len()));
    let cached = cache_dir.join(format!("{key:016x}.cooked"));
    if let Ok(bytes) = std::fs::read(&cached) {
        if let Some(kind) = read_header(&bytes) {
            return Ok((kind, cached));
        }
    }
    let (kind, data) = match plan(relative) {
        Plan::Model => {
            let scene = russimp_ng::scene::Scene::from_file(
                &source.to_string_lossy(),
                crate::core::model::model_post_process(),
            )
            .map_err(|e| e.to_string())?;
            (Kind::Scene, super::cooked_scene::write(&scene))
        }
        Plan::Image => {
            let image = image::open(&source).map_err(|e| e.to_string())?.to_rgba8();
            let (width, height) = image.dimensions();
            let (levels, data) = encode_bc7(image);
            (Kind::Bc7 { width, height, mips: levels }, data)
        }
        Plan::Loose => return Err("not cooked".into()),
    };
    let mut bytes = write_header(kind);
    bytes.extend_from_slice(&data);
    // Written aside and renamed: a cut file is never taken for a cooked one.
    let partial = cached.with_extension("partial");
    std::fs::write(&partial, &bytes).map_err(|e| format!("{}: {e}", partial.display()))?;
    std::fs::rename(&partial, &cached).map_err(|e| format!("{}: {e}", cached.display()))?;
    fresh.fetch_add(1, Ordering::Relaxed);
    Ok((kind, cached))
}

fn write_header(kind: Kind) -> Vec<u8> {
    let mut header = vec![0u8; HEADER];
    header[0..4].copy_from_slice(b"FCKD");
    let (tag, a, b, c) = match kind {
        Kind::Scene => (1u32, 0, 0, 0),
        Kind::Bc7 { width, height, mips } => (2, width, height, mips),
        Kind::Raw => (3, 0, 0, 0),
    };
    for (i, v) in [tag, a, b, c, CACHE_VERSION].iter().enumerate() {
        header[4 + i * 4..8 + i * 4].copy_from_slice(&v.to_le_bytes());
    }
    header
}

fn read_header(bytes: &[u8]) -> Option<Kind> {
    if bytes.len() < HEADER || &bytes[0..4] != b"FCKD" {
        return None;
    }
    let v = |i: usize| u32::from_le_bytes(bytes[4 + i * 4..8 + i * 4].try_into().unwrap());
    if v(4) != CACHE_VERSION {
        return None;
    }
    match v(0) {
        1 => Some(Kind::Scene),
        2 => Some(Kind::Bc7 { width: v(1), height: v(2), mips: v(3) }),
        3 => Some(Kind::Raw),
        _ => None,
    }
}

fn fnv(text: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in text.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// BC7 blocks of `image` and of each mip level below it, the largest
/// first: (levels, bytes). Each level is the one above it halved (a
/// triangle filter), as the GPU would make it.
pub fn encode_bc7(image: RgbaImage) -> (u32, Vec<u8>) {
    let (width, height) = image.dimensions();
    let levels = crate::core::renderer::texture::decode::Bc7Image::level_count(width, height);
    let opaque = image.pixels().all(|p| p.0[3] == 255);
    let settings = if opaque {
        intel_tex_2::bc7::opaque_basic_settings()
    } else {
        intel_tex_2::bc7::alpha_basic_settings()
    };
    let mut out = Vec::new();
    let mut level_image = image;
    for level in 0..levels {
        let (w, h) = ((width >> level).max(1), (height >> level).max(1));
        if level > 0 {
            level_image = image::imageops::resize(&level_image, w, h, image::imageops::FilterType::Triangle);
        }
        out.extend_from_slice(&encode_level(&level_image, &settings));
    }
    (levels, out)
}

/// One level's blocks; the image is padded to whole blocks by repeating
/// its last row and column.
fn encode_level(image: &RgbaImage, settings: &intel_tex_2::bc7::EncodeSettings) -> Vec<u8> {
    let (w, h) = image.dimensions();
    let (pw, ph) = (w.div_ceil(4) * 4, h.div_ceil(4) * 4);
    let padded;
    let source = if (pw, ph) == (w, h) {
        image
    } else {
        padded = RgbaImage::from_fn(pw, ph, |x, y| *image.get_pixel(x.min(w - 1), y.min(h - 1)));
        &padded
    };
    let surface = intel_tex_2::RgbaSurface { data: source.as_raw(), width: pw, height: ph, stride: pw * 4 };
    intel_tex_2::bc7::compress_blocks(settings, &surface)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::renderer::texture::decode::Bc7Image;

    #[test]
    fn bc7_levels_have_the_sizes_the_upload_expects() {
        let image = RgbaImage::from_fn(30, 17, |x, y| image::Rgba([(x * 8) as u8, (y * 15) as u8, 90, 255]));
        let (levels, bytes) = encode_bc7(image.clone());
        assert_eq!(levels, 5);
        let total: usize = (0..levels).map(|l| Bc7Image::level_size(30, 17, l)).sum();
        assert_eq!(bytes.len(), total);
        let back = Bc7Image::from_levels(30, 17, levels, &bytes).unwrap();
        let pixels = back.decode();
        assert_eq!(pixels.dimensions(), (30, 17));
        // Close to the original (BC7 is lossy, but not by much on a ramp).
        let error: u32 = pixels
            .pixels()
            .zip(image.pixels())
            .map(|(a, b)| (0..3).map(|c| (a.0[c] as i32 - b.0[c] as i32).unsigned_abs()).max().unwrap())
            .max()
            .unwrap();
        assert!(error <= 12, "max error {error}");
    }

    /// How long reading every model (with its maps decoded) of the project
    /// in FERRITE_PACK_PROJECT takes from its loose files, and from the
    /// pack FERRITE_PACK_FILE made of it. Run alone (it changes the working
    /// directory).
    #[test]
    #[ignore]
    fn loose_files_against_the_pack() {
        let (Ok(project), Ok(pack_file)) = (std::env::var("FERRITE_PACK_PROJECT"), std::env::var("FERRITE_PACK_FILE")) else {
            eprintln!("set FERRITE_PACK_PROJECT and FERRITE_PACK_FILE");
            return;
        };
        std::env::set_current_dir(&project).unwrap();
        let mut files = Vec::new();
        collect(Path::new("assets"), Path::new(""), &mut files);
        let models: Vec<String> = files.into_iter().filter(|f| matches!(plan(f), Plan::Model)).collect();
        let time = |label: &str| {
            let start = std::time::Instant::now();
            let mut bytes = 0usize;
            for model in &models {
                match crate::core::model::PreparedModel::load(model) {
                    Ok(prepared) => bytes += prepared.decoded_bytes(),
                    Err(e) => eprintln!("{model}: {e}"),
                }
            }
            let took = start.elapsed();
            eprintln!("{label}: {} models in {:.2} s, {:.0} MB of maps for the GPU", models.len(), took.as_secs_f64(), bytes as f64 / 1e6);
        };
        time("loose files");
        super::super::pack::open(Path::new(&pack_file)).unwrap();
        time("asset pack");
        super::super::pack::close();
    }

    #[test]
    fn a_project_cooks_into_a_pack_the_loaders_read() {
        let _guard = super::super::pack::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("ferrite-cook-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let models = dir.join("assets/models/thing");
        std::fs::create_dir_all(&models).unwrap();
        std::fs::create_dir_all(dir.join("assets/textures/terrain/grass")).unwrap();
        super::super::test_models::write_gltf(&models.join("thing.gltf"), &[("box", &[])], Some("thing_albedo.png"));
        super::super::test_models::write_png(&models.join("thing_albedo.png"));
        super::super::test_models::write_png(&dir.join("assets/textures/terrain/grass/grass_albedo.png"));
        std::fs::write(models.join("thing.gltf.asset.ron"), "()").unwrap();
        let pack_path = dir.join("assets.pack");
        let cooked = cook_project(&dir, &["assets"], &pack_path, &|_, _, _| {}).unwrap();
        assert!(cooked.failed.is_empty(), "{:?}", cooked.failed);
        assert!(cooked.packed.contains(&"assets/models/thing/thing.gltf".to_string()));
        assert!(cooked.packed.contains(&"assets/models/thing/thing_albedo.png".to_string()));
        // Terrain images and sidecars stay loose.
        assert!(cooked.loose.contains(&"assets/textures/terrain/grass/grass_albedo.png".to_string()));
        assert!(cooked.loose.contains(&"assets/models/thing/thing.gltf.asset.ron".to_string()));
        assert_eq!(cooked.cooked, 2);
        // Again: all from the cache.
        let again = cook_project(&dir, &["assets"], &pack_path, &|_, _, _| {}).unwrap();
        assert_eq!(again.cooked, 0);

        // Read through the pack alone: the paths do not exist here.
        super::super::pack::open(&pack_path).unwrap();
        let scene = crate::core::model::read_scene("assets/models/thing/../thing/thing.gltf");
        let image = crate::core::renderer::texture::decode::decode_file(Path::new("assets/models/thing/thing_albedo.png"));
        let listed = super::super::vfs::files_in(Path::new("assets/models/thing"));
        super::super::pack::close();
        assert!(scene.is_ok_and(|s| !s.meshes.is_empty()));
        assert!(matches!(image, Ok(crate::core::renderer::texture::decode::Image::Bc7(_))));
        assert!(listed.iter().any(|p| p.ends_with("thing_albedo.png")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
