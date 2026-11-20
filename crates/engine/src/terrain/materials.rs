//! Textures of a terrain's surface materials, for sources whose fragment
//! program blends textured layers (see `TerrainSource::load_textures`):
//! each layer's scan loaded from `assets/textures/terrain/<layer>/` as
//! array textures, one layer per material, one array per kind of map
//! (albedo, normal, and roughness/occlusion/height), and the uniforms
//! `tileSize[i]`, `heightDepth[i]`, `tinted[i]`, `turn[i]` and
//! `hasTerrainTextures` set.
//!
//! Layers without a stand-in are needed (without them the ground is drawn
//! in plain colours); a project without one of the others gets its
//! stand-in's textures in its place. The snow's scan (see `load_snow`) is
//! the weather's, for every terrain.

use std::path::{Path, PathBuf};

use image::{imageops::FilterType, RgbaImage};

use crate::core::{
    model::material_maps::{self, MapKind},
    renderer::{shader::Shader, texture::Texture},
};

const FOLDER: &str = "assets/textures/terrain";

pub struct TerrainLayer {
    /// Folder below `assets/textures/terrain/`.
    pub folder: &'static str,
    /// Metres the texture covers, from the scan's physical size.
    pub tile_size: f32,
    /// Metres between the lowest and highest point of the height map, from
    /// the scan.
    pub height_depth: f32,
    /// The layer whose textures stand in when this one's are missing;
    /// `None`: needed.
    pub stand_in: Option<usize>,
    /// Whether the biomes' ground colour tints it (grass does).
    pub tinted: bool,
    /// How far the shader may turn its copies to hide the tiling, as a
    /// share of a full turn either way (0: never, as a road's tracks
    /// follow it; small where the texture has a direction).
    pub turn: f32,
}

pub const fn layer(folder: &'static str, tile_size: f32, height_depth: f32, stand_in: Option<usize>) -> TerrainLayer {
    TerrainLayer {
        folder,
        tile_size,
        height_depth,
        stand_in,
        tinted: false,
        turn: 0.5,
    }
}

/// What a layer leaves out: medium roughness, no occlusion, flat, half
/// height.
const DEFAULT_ROUGHNESS: u8 = 204;
const DEFAULT_NORMAL: [u8; 4] = [128, 128, 255, 255];

/// The maps of one material scan.
pub struct MaterialImages {
    pub albedo: RgbaImage,
    pub normal: Option<RgbaImage>,
    pub roughness: Option<RgbaImage>,
    /// Glossiness, used inverted when there is no roughness map.
    pub gloss: Option<RgbaImage>,
    pub occlusion: Option<RgbaImage>,
    pub height: Option<RgbaImage>,
}

/// Load the layers' textures and set the shader's terrain uniforms. Returns
/// the albedo, normal and surface (roughness, occlusion, height) arrays for
/// texture units 0, 1 and 2, or nothing — and the shader falls back to
/// plain colours — when a layer's colour map is missing.
pub fn load(shader: &Shader, layers: &[TerrainLayer]) -> Vec<Texture> {
    debug_assert!(
        layers.iter().enumerate().all(|(i, l)| l.stand_in.is_none_or(|s| s < i)),
        "stand-ins come before what they stand in for"
    );
    let loaded: Vec<Option<MaterialImages>> = layers
        .iter()
        .map(|layer| load_images(&Path::new(FOLDER).join(layer.folder), layer.stand_in.is_none()))
        .collect();
    if layers.is_empty() || layers.iter().zip(&loaded).any(|(layer, images)| layer.stand_in.is_none() && images.is_none()) {
        shader.set_uniform_1i("hasTerrainTextures", 0);
        return Vec::new();
    }
    // Missing layers take their stand-in's textures, size and tint (or
    // its stand-in's, when that is missing too).
    let mut from = vec![0usize; layers.len()];
    for (index, layer) in layers.iter().enumerate() {
        from[index] = match (&loaded[index], layer.stand_in) {
            (None, Some(stand_in)) => {
                let source = from[stand_in];
                log::info!("terrain: no {} textures, using {}'s", layer.folder, layers[source].folder);
                source
            }
            _ => index,
        };
    }
    for (index, &source) in from.iter().enumerate() {
        let layer = &layers[source];
        shader.set_uniform_1f(&format!("tileSize[{index}]"), layer.tile_size);
        shader.set_uniform_1f(&format!("heightDepth[{index}]"), layer.height_depth);
        shader.set_uniform_1f(&format!("tinted[{index}]"), if layer.tinted { 1.0 } else { 0.0 });
        // Its own direction, even when another layer's textures stand in.
        shader.set_uniform_1f(&format!("turn[{index}]"), layers[index].turn);
    }
    let images: Vec<&MaterialImages> = from
        .iter()
        .map(|&source| loaded[source].as_ref().expect("needed layers are loaded"))
        .collect();
    let (width, height) = images[0].albedo.dimensions();
    let fit = |image: &RgbaImage| {
        if image.dimensions() == (width, height) {
            image.clone()
        } else {
            image::imageops::resize(image, width, height, FilterType::Triangle)
        }
    };
    let pixels = (width * height) as usize;

    let albedo: Vec<Vec<u8>> = images.iter().map(|l| fit(&l.albedo).into_raw()).collect();
    let normal: Vec<Vec<u8>> = images
        .iter()
        .map(|l| match &l.normal {
            Some(normal) => fit(normal).into_raw(),
            None => DEFAULT_NORMAL.repeat(pixels),
        })
        .collect();
    let surface: Vec<Vec<u8>> = images
        .iter()
        .map(|l| {
            let channel = |image: &Option<RgbaImage>| image.as_ref().map(|i| fit(i).into_raw());
            let roughness = channel(&l.roughness);
            let gloss = channel(&l.gloss);
            let occlusion = channel(&l.occlusion);
            let height = channel(&l.height);
            let mut out = vec![255u8; pixels * 4];
            for (i, texel) in out.chunks_exact_mut(4).enumerate() {
                let at = i * 4;
                texel[0] = match (&roughness, &gloss) {
                    (Some(r), _) => r[at],
                    (None, Some(g)) => 255 - g[at],
                    (None, None) => DEFAULT_ROUGHNESS,
                };
                texel[1] = occlusion.as_ref().map_or(255, |o| o[at]);
                texel[2] = height.as_ref().map_or(128, |h| h[at]);
            }
            out
        })
        .collect();

    let mut textures = Vec::with_capacity(3);
    debug_assert_eq!(albedo.len(), layers.len());
    for (layers, srgb) in [(albedo, true), (normal, false), (surface, false)] {
        let mut texture = Texture::new();
        texture.load_array(width, height, &layers, srgb);
        textures.push(texture);
    }
    shader.set_uniform_1i("hasTerrainTextures", 1);
    textures
}

/// Folder below `assets/textures/terrain/` with the snow's scan.
pub const SNOW_FOLDER: &str = "snow";

/// The snow's textures from `assets/textures/terrain/snow/`, as one-layer
/// arrays: colour with roughness in alpha (sRGB colour), and the normal
/// with occlusion in alpha. Nothing when there is no colour map.
pub fn load_snow() -> Option<[Texture; 2]> {
    let layer = load_images(&Path::new(FOLDER).join(SNOW_FOLDER), true)?;
    let (width, height) = layer.albedo.dimensions();
    let fit = |image: &RgbaImage| {
        if image.dimensions() == (width, height) {
            image.clone()
        } else {
            image::imageops::resize(image, width, height, FilterType::Triangle)
        }
    };
    let roughness = layer.roughness.as_ref().map(|i| fit(i).into_raw());
    let gloss = layer.gloss.as_ref().map(|i| fit(i).into_raw());
    let normal = layer.normal.as_ref().map(|i| fit(i).into_raw());
    let occlusion = layer.occlusion.as_ref().map(|i| fit(i).into_raw());
    let mut colour = layer.albedo.into_raw();
    let mut surface = vec![0u8; colour.len()];
    for i in 0..(width * height) as usize {
        let at = i * 4;
        colour[at + 3] = match (&roughness, &gloss) {
            (Some(r), _) => r[at],
            (None, Some(g)) => 255 - g[at],
            (None, None) => DEFAULT_ROUGHNESS,
        };
        let n = normal.as_ref().map_or(DEFAULT_NORMAL, |n| [n[at], n[at + 1], n[at + 2], 255]);
        surface[at..at + 3].copy_from_slice(&n[..3]);
        surface[at + 3] = occlusion.as_ref().map_or(255, |o| o[at]);
    }
    let mut textures = [Texture::new(), Texture::new()];
    textures[0].load_array(width, height, &[colour], true);
    textures[1].load_array(width, height, &[surface], false);
    Some(textures)
}

/// The maps of the scan in `folder` (found by their names, see
/// `material_maps`), if it has a colour map; `needed`: say so when it has
/// none.
pub fn load_images(folder: &Path, needed: bool) -> Option<MaterialImages> {
    let files: Vec<PathBuf> = std::fs::read_dir(folder)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect();
    let names: Vec<String> = files
        .iter()
        .filter_map(|p| p.file_name()?.to_str().map(str::to_lowercase))
        .collect();
    let prefix = common_prefix(&names);
    let maps = material_maps::find_maps(&files, &[prefix]);
    let read = |kind: MapKind| {
        let (_, path) = maps.iter().find(|(k, _)| *k == kind)?;
        match image::open(path) {
            Ok(image) => Some(image.to_rgba8()),
            Err(e) => {
                log::warn!("terrain: could not read {}: {e}", path.display());
                None
            }
        }
    };
    let Some(albedo) = read(MapKind::BaseColor) else {
        if !needed {
            return None;
        }
        log::warn!(
            "terrain: no colour map in {}, drawing the terrain in plain colours",
            folder.display()
        );
        return None;
    };
    Some(MaterialImages {
        albedo,
        normal: read(MapKind::Normal),
        roughness: read(MapKind::Roughness),
        gloss: read(MapKind::Gloss),
        occlusion: read(MapKind::Occlusion),
        height: read(MapKind::Height),
    })
}

/// Longest prefix all `names` share, cut back to the last `_`, ` ` or `-`
/// so it ends between words (`rock_rough_1k_` of `rock_rough_1k_ao.jpg`
/// and `rock_rough_1k_roughness.jpg`).
fn common_prefix(names: &[String]) -> String {
    let Some(first) = names.first() else {
        return String::new();
    };
    let mut length = first.len();
    for name in &names[1..] {
        length = first
            .bytes()
            .zip(name.bytes())
            .take(length)
            .take_while(|(a, b)| a == b)
            .count();
    }
    // Equal bytes may end inside a character (`é` and `è` share a byte).
    while !first.is_char_boundary(length) {
        length -= 1;
    }
    let shared = &first[..length];
    match shared.rfind(['_', ' ', '-']) {
        Some(end) => shared[..=end].to_string(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_common_prefix_never_cuts_a_character() {
        let names = ["café_ao.png".to_string(), "cafè_basecolor.png".to_string()];
        assert_eq!(common_prefix(&names), "");
        let names = ["mé_x_ao.png".to_string(), "mé_x_rough.png".to_string()];
        assert_eq!(common_prefix(&names), "mé_x_");
    }

    #[test]
    fn the_common_prefix_ends_between_words() {
        let names = vec![
            "rock_rough_olqgk0_1k_ao.jpg".to_string(),
            "rock_rough_olqgk0_1k_basecolor.jpg".to_string(),
            "rock_rough_olqgk0_1k_roughness.jpg".to_string(),
        ];
        assert_eq!(common_prefix(&names), "rock_rough_olqgk0_1k_");
        // The words of the prefix ("rough") do not count as a map kind.
        assert_eq!(
            material_maps::map_kind("Rock_Rough_olqgK0_1K_BaseColor.jpg", &common_prefix(&names)),
            Some(MapKind::BaseColor)
        );
    }
}
