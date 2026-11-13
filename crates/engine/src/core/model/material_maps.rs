//! Finding a material's texture maps by file name, for files whose
//! materials do not refer to their textures (common in FBX exports and
//! scanned asset packs). Used by the model loader and by the importer, so
//! both pick up the same files.

use std::path::{Path, PathBuf};

use crate::core::assets::MaterialMap;

const IMAGE_EXTENSIONS: [&str; 7] = ["png", "jpg", "jpeg", "tga", "bmp", "tif", "tiff"];

/// What a texture file holds, judged by its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MapKind {
    BaseColor,
    Normal,
    Roughness,
    /// Glossiness: roughness inverted.
    Gloss,
    Metallic,
    Occlusion,
    Opacity,
}

impl MapKind {
    /// The material slot the map fills (a gloss map fills roughness).
    pub fn slot(self) -> Option<MaterialMap> {
        match self {
            MapKind::BaseColor => Some(MaterialMap::BaseColor),
            MapKind::Normal => Some(MaterialMap::Normal),
            MapKind::Roughness | MapKind::Gloss => Some(MaterialMap::Roughness),
            MapKind::Metallic => Some(MaterialMap::Metallic),
            MapKind::Occlusion => Some(MaterialMap::Occlusion),
            MapKind::Opacity => None,
        }
    }
}

/// Name prefixes the maps of a material may start with, most specific
/// first: the material's name without a `_Mat` / `_Material` suffix, then
/// the model file's name (scanned assets name every map after the file).
pub fn name_prefixes(material_name: Option<&str>, model_stem: &str) -> Vec<String> {
    let mut prefixes = Vec::new();
    if let Some(name) = material_name {
        let lower = name.to_lowercase();
        let prefix = lower
            .trim_end_matches("_material")
            .trim_end_matches("_mat")
            .trim_end_matches('_')
            .trim();
        if !prefix.is_empty() {
            prefixes.push(prefix.to_string());
        }
    }
    let stem = model_stem.trim().to_lowercase();
    if !stem.is_empty() && !prefixes.contains(&stem) {
        prefixes.push(stem);
    }
    prefixes
}

/// What `file_name` holds, judged by the part after `prefix`. Words like
/// `BaseColor`, `Normal`, `Roughness`, `AO` are recognised; maps the
/// renderer does not use (bump, cavity, specular, ...) give `None`.
pub fn map_kind(file_name: &str, prefix: &str) -> Option<MapKind> {
    let lower = file_name.to_lowercase();
    let path = Path::new(&lower);
    let extension = path.extension()?.to_str()?;
    if !IMAGE_EXTENSIONS.contains(&extension) {
        return None;
    }
    let stem = path.file_stem()?.to_str()?;
    let rest = stem.strip_prefix(prefix)?;
    let words: Vec<&str> = rest
        .split(|c: char| c == '_' || c == ' ' || c == '-' || c == '.')
        .filter(|w| !w.is_empty())
        .collect();
    words.iter().enumerate().find_map(|(i, word)| {
        let next = words.get(i + 1).copied().unwrap_or("");
        match *word {
            "basecolor" | "albedo" | "diffuse" | "diff" | "color" | "colour" | "col" => {
                Some(MapKind::BaseColor)
            }
            "base" if next == "color" || next == "colour" => Some(MapKind::BaseColor),
            "normal" | "normals" | "nrm" | "nor" | "norm" | "normalgl" => Some(MapKind::Normal),
            "roughness" | "rough" | "rgh" => Some(MapKind::Roughness),
            "gloss" | "glossiness" => Some(MapKind::Gloss),
            "metallic" | "metalness" | "metal" => Some(MapKind::Metallic),
            "ao" | "ambientocclusion" | "occlusion" => Some(MapKind::Occlusion),
            "ambient" if next == "occlusion" => Some(MapKind::Occlusion),
            "opacity" | "alpha" | "mask" | "transparency" => Some(MapKind::Opacity),
            _ => None,
        }
    })
}

/// The maps among `files` for the first prefix that has any. Within one
/// prefix the first file of each kind (by name) wins, and a roughness map
/// wins over a gloss map.
pub fn find_maps(files: &[PathBuf], prefixes: &[String]) -> Vec<(MapKind, PathBuf)> {
    let mut sorted: Vec<&PathBuf> = files.iter().collect();
    sorted.sort();
    for prefix in prefixes {
        let mut found: Vec<(MapKind, PathBuf)> = Vec::new();
        for file in &sorted {
            let Some(name) = file.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let Some(kind) = map_kind(name, prefix) else {
                continue;
            };
            if !found.iter().any(|(k, _)| *k == kind) {
                found.push((kind, (*file).clone()));
            }
        }
        if found.iter().any(|(k, _)| *k == MapKind::Roughness) {
            found.retain(|(k, _)| *k != MapKind::Gloss);
        }
        if !found.is_empty() {
            return found;
        }
    }
    Vec::new()
}

/// Every image file in `dir` whose name starts with one of `prefixes`
/// (case-insensitive), sorted.
pub fn images_with_prefix(dir: &Path, prefixes: &[String]) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|path| {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                return false;
            };
            let lower = name.to_lowercase();
            let is_image = Path::new(&lower)
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| IMAGE_EXTENSIONS.contains(&e));
            path.is_file() && is_image && prefixes.iter().any(|p| lower.starts_with(p.as_str()))
        })
        .collect();
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(|n| PathBuf::from("dir").join(n)).collect()
    }

    #[test]
    fn scanned_asset_maps_are_found_by_the_model_file_name() {
        let files = paths(&[
            "Rock_Low_1K_AO.jpg",
            "Rock_Low_1K_BaseColor.jpg",
            "Rock_Low_1K_Bump.jpg",
            "Rock_Low_1K_Cavity.jpg",
            "Rock_Low_1K_Gloss.jpg",
            "Rock_Low_1K_Normal.jpg",
            "Rock_Low_1K_Roughness.jpg",
            "Rock_Low_1K_Specular.jpg",
        ]);
        let prefixes = name_prefixes(Some("Default_Material1"), "Rock_Low");
        let mut maps = find_maps(&files, &prefixes);
        maps.sort_by_key(|(k, _)| format!("{k:?}"));
        let names: Vec<(MapKind, &str)> = maps
            .iter()
            .map(|(k, p)| (*k, p.file_name().unwrap().to_str().unwrap()))
            .collect();
        assert_eq!(
            names,
            vec![
                (MapKind::BaseColor, "Rock_Low_1K_BaseColor.jpg"),
                (MapKind::Normal, "Rock_Low_1K_Normal.jpg"),
                (MapKind::Occlusion, "Rock_Low_1K_AO.jpg"),
                (MapKind::Roughness, "Rock_Low_1K_Roughness.jpg"),
            ]
        );
    }

    #[test]
    fn a_gloss_map_is_used_when_there_is_no_roughness_map() {
        let files = paths(&["Rock_Gloss.png", "Rock_Albedo.png"]);
        let maps = find_maps(&files, &["rock".to_string()]);
        assert!(maps.contains(&(MapKind::Gloss, PathBuf::from("dir/Rock_Gloss.png"))));
    }

    #[test]
    fn the_material_name_wins_over_the_file_name() {
        let files = paths(&["Bark_albedo.png", "Bark_normal.png", "Pack_Color.png"]);
        let prefixes = name_prefixes(Some("Bark_Mat"), "Pack");
        let maps = find_maps(&files, &prefixes);
        assert_eq!(maps.len(), 2);
        assert!(maps.iter().all(|(_, p)| p.to_str().unwrap().contains("Bark")));
    }

    #[test]
    fn common_spellings_are_recognised() {
        assert_eq!(map_kind("pine_bark_diff_1k.jpg", "pine_bark"), Some(MapKind::BaseColor));
        assert_eq!(map_kind("pine_bark_nor_gl_1k.jpg", "pine_bark"), Some(MapKind::Normal));
        assert_eq!(map_kind("Cluster_Opacity.png", "cluster"), Some(MapKind::Opacity));
        assert_eq!(map_kind("bark_ambientocclusion.png", "bark"), Some(MapKind::Occlusion));
        assert_eq!(map_kind("Metal_Base_Color.png", "metal"), Some(MapKind::BaseColor));
        assert_eq!(map_kind("rock_metallic.png", "rock"), Some(MapKind::Metallic));
        assert_eq!(map_kind("rock_height.png", "rock"), None);
        assert_eq!(map_kind("rock_normal.txt", "rock"), None);
        assert_eq!(map_kind("other_normal.png", "rock"), None);
    }
}
