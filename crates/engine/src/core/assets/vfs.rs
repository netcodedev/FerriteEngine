//! Files as the loaders see them: loose files on disk, and what the asset
//! pack holds (see `pack`). Without a pack this is plain `std::fs`.

use std::path::{Path, PathBuf};

use super::pack;

/// Whether `path` is a file on disk or in the pack.
pub fn is_file(path: &Path) -> bool {
    path.is_file() || pack::entry(path).is_some()
}

/// The files in `folder` (not below it): on disk and in the pack, each
/// once, sorted.
pub fn files_in(folder: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(folder)
        .map(|entries| entries.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect())
        .unwrap_or_default();
    for packed in pack::files_in(folder) {
        if !files.contains(&packed) {
            files.push(packed);
        }
    }
    files.sort();
    files
}

/// The bytes of `path`: the loose file, or else a raw entry of the pack.
pub fn read(path: &Path) -> Result<Vec<u8>, String> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(bytes),
        Err(e) => match pack::entry(path) {
            Some(entry) if entry.kind == pack::Kind::Raw => pack::read(&entry),
            _ => Err(format!("{}: {e}", path.display())),
        },
    }
}

/// The Assimp scene of the model file `path`: the pack's cooked copy if it
/// has one, else the file read with Assimp and `post_process`. A cooked
/// scene was read with the model loader's post-processing (see
/// `model::read_scene`), which changes no node, material or animation.
pub fn read_scene(
    path: &Path,
    post_process: Vec<russimp_ng::scene::PostProcess>,
) -> Result<russimp_ng::scene::Scene, String> {
    if let Some(entry) = pack::entry(path).filter(|e| e.kind == pack::Kind::Scene) {
        let bytes = pack::read(&entry)?;
        return super::cooked_scene::read(&bytes).map_err(|e| format!("{}: {e}", path.display()));
    }
    russimp_ng::scene::Scene::from_file(&path.to_string_lossy(), post_process).map_err(|e| e.to_string())
}
