//! Textures of the terrain's surface materials (grass, sand, rock), loaded
//! from `assets/textures/terrain/<layer>/` as array textures: one layer per
//! material, one array per kind of map.

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
}

/// In the order the terrain shader's `GRASS`, `SAND`, `ROCK` index them.
pub const LAYERS: [TerrainLayer; 3] = [
    TerrainLayer {
        folder: "grass",
        tile_size: 1.0,
        height_depth: 0.04,
    },
    TerrainLayer {
        folder: "sand",
        tile_size: 2.0,
        height_depth: 0.031,
    },
    TerrainLayer {
        folder: "rock",
        tile_size: 0.83,
        height_depth: 0.059,
    },
];

/// What a layer leaves out: medium roughness, no occlusion, flat, half
/// height.
const DEFAULT_ROUGHNESS: u8 = 204;
const DEFAULT_NORMAL: [u8; 4] = [128, 128, 255, 255];

struct LayerImages {
    albedo: RgbaImage,
    normal: Option<RgbaImage>,
    roughness: Option<RgbaImage>,
    /// Glossiness, used inverted when there is no roughness map.
    gloss: Option<RgbaImage>,
    occlusion: Option<RgbaImage>,
    height: Option<RgbaImage>,
}

/// Load the layers' textures and set the shader's terrain uniforms. Returns
/// the albedo, normal and surface (roughness, occlusion, height) arrays for
/// texture units 0, 1 and 2, or nothing — and the shader falls back to
/// plain colours — when a layer's colour map is missing.
pub fn load(shader: &Shader) -> Vec<Texture> {
    shader.bind();
    for (index, layer) in LAYERS.iter().enumerate() {
        shader.set_uniform_1f(&format!("tileSize[{index}]"), layer.tile_size);
        shader.set_uniform_1f(&format!("heightDepth[{index}]"), layer.height_depth);
    }
    let layers: Option<Vec<LayerImages>> = LAYERS
        .iter()
        .map(|layer| load_layer(&Path::new(FOLDER).join(layer.folder)))
        .collect();
    let Some(layers) = layers else {
        shader.set_uniform_1i("hasTerrainTextures", 0);
        return Vec::new();
    };
    let (width, height) = layers[0].albedo.dimensions();
    let fit = |image: &RgbaImage| {
        if image.dimensions() == (width, height) {
            image.clone()
        } else {
            image::imageops::resize(image, width, height, FilterType::Triangle)
        }
    };
    let pixels = (width * height) as usize;

    let albedo: Vec<Vec<u8>> = layers.iter().map(|l| fit(&l.albedo).into_raw()).collect();
    let normal: Vec<Vec<u8>> = layers
        .iter()
        .map(|l| match &l.normal {
            Some(normal) => fit(normal).into_raw(),
            None => DEFAULT_NORMAL.repeat(pixels),
        })
        .collect();
    let surface: Vec<Vec<u8>> = layers
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
    for (layers, srgb) in [(albedo, true), (normal, false), (surface, false)] {
        let mut texture = Texture::new();
        texture.load_array(width, height, &layers, srgb);
        textures.push(texture);
    }
    shader.bind();
    shader.set_uniform_1i("hasTerrainTextures", 1);
    textures
}

fn load_layer(folder: &Path) -> Option<LayerImages> {
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
        log::warn!(
            "terrain: no colour map in {}, drawing the terrain in plain colours",
            folder.display()
        );
        return None;
    };
    Some(LayerImages {
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
