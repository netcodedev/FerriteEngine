//! Textures of the terrain's surface materials (grass, sand, rock, forest
//! floor, dry grass, gravel, road, dirt, jungle floor, tundra moss, mud,
//! cave rock), loaded from
//! `assets/textures/terrain/<layer>/` as array textures: one layer per
//! material, one array per kind of map.
//!
//! Grass, sand and rock are needed (without them the terrain is drawn in
//! plain colours); a project without one of the others gets its stand-in's
//! textures in its place.

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

const fn layer(folder: &'static str, tile_size: f32, height_depth: f32, stand_in: Option<usize>) -> TerrainLayer {
    TerrainLayer {
        folder,
        tile_size,
        height_depth,
        stand_in,
        tinted: false,
        turn: 0.5,
    }
}

pub const GRASS: usize = 0;
pub const SAND: usize = 1;
pub const ROCK: usize = 2;
pub const FOREST: usize = 3;
pub const DIRT: usize = 7;

/// In the order the terrain shader's `GRASS`, `SAND`, `ROCK`, `FOREST`,
/// `DRY`, `GRAVEL`, `ROAD`, `DIRT`, `JUNGLE`, `TUNDRA`, `MUD`, `CAVE_ROCK`
/// index them. Stand-ins come before what they stand in for.
pub const LAYERS: [TerrainLayer; 12] = [
    TerrainLayer {
        tinted: true,
        ..layer("grass", 1.0, 0.04, None)
    },
    layer("sand", 2.0, 0.031, None),
    layer("rock", 0.83, 0.059, None),
    layer("forest_floor", 1.5, 0.057, Some(GRASS)),
    layer("dry_grass", 1.0, 0.035, Some(GRASS)),
    layer("gravel", 0.39, 0.05, Some(SAND)),
    TerrainLayer {
        turn: 0.0,
        ..layer("road", 2.0, 0.029, Some(SAND))
    },
    // Its tracks run one way: turned a little only.
    TerrainLayer {
        turn: 0.04,
        ..layer("dirt", 1.0, 0.052, Some(SAND))
    },
    layer("jungle_floor", 2.0, 0.018, Some(FOREST)),
    layer("tundra", 4.0, 0.032, Some(GRASS)),
    layer("mud", 0.42, 0.049, Some(DIRT)),
    layer("cave_rock", 1.0, 0.151, Some(ROCK)),
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
    let loaded: Vec<Option<LayerImages>> = LAYERS
        .iter()
        .map(|layer| load_layer(&Path::new(FOLDER).join(layer.folder), layer.stand_in.is_none()))
        .collect();
    if LAYERS.iter().zip(&loaded).any(|(layer, images)| layer.stand_in.is_none() && images.is_none()) {
        shader.set_uniform_1i("hasTerrainTextures", 0);
        return Vec::new();
    }
    // Missing layers take their stand-in's textures, size and tint (or
    // its stand-in's, when that is missing too).
    let mut from = [0usize; LAYERS.len()];
    for (index, layer) in LAYERS.iter().enumerate() {
        from[index] = match (&loaded[index], layer.stand_in) {
            (None, Some(stand_in)) => {
                let source = from[stand_in];
                log::info!("terrain: no {} textures, using {}'s", layer.folder, LAYERS[source].folder);
                source
            }
            _ => index,
        };
    }
    for (index, &source) in from.iter().enumerate() {
        let layer = &LAYERS[source];
        shader.set_uniform_1f(&format!("tileSize[{index}]"), layer.tile_size);
        shader.set_uniform_1f(&format!("heightDepth[{index}]"), layer.height_depth);
        shader.set_uniform_1f(&format!("tinted[{index}]"), if layer.tinted { 1.0 } else { 0.0 });
        // Its own direction, even when another layer's textures stand in.
        shader.set_uniform_1f(&format!("turn[{index}]"), LAYERS[index].turn);
    }
    let layers: Vec<&LayerImages> = from
        .iter()
        .map(|&source| loaded[source].as_ref().expect("needed layers are loaded"))
        .collect();
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
    debug_assert_eq!(albedo.len(), LAYERS.len());
    for (layers, srgb) in [(albedo, true), (normal, false), (surface, false)] {
        let mut texture = Texture::new();
        texture.load_array(width, height, &layers, srgb);
        textures.push(texture);
    }
    shader.bind();
    shader.set_uniform_1i("hasTerrainTextures", 1);
    textures
}

/// The road layer's colour and normal maps, as images (for what draws a
/// road apart from the ground: the bridges).
pub fn road_images() -> Option<(RgbaImage, Option<RgbaImage>)> {
    let road = LAYERS.iter().find(|layer| layer.folder == "road")?;
    let images = load_layer(&Path::new(FOLDER).join(road.folder), false)?;
    Some((images.albedo, images.normal))
}

/// The bridges' stone: colour, normal, roughness and occlusion maps from
/// `assets/textures/bridge/` (one repeat a bridge's stone tile, see
/// `bridges::STONE_TILE`), if it has a colour map.
pub fn bridge_images() -> Option<[Option<RgbaImage>; 4]> {
    let images = load_layer(Path::new(BRIDGE_FOLDER), false)?;
    // Gloss inverted where there is no roughness.
    let roughness = images.roughness.or_else(|| {
        images.gloss.map(|mut gloss| {
            gloss.pixels_mut().for_each(|p| p.0[..3].iter_mut().for_each(|c| *c = 255 - *c));
            gloss
        })
    });
    Some([Some(images.albedo), images.normal, roughness, images.occlusion])
}

/// Where the bridges' textures lie (see `bridge_images`).
const BRIDGE_FOLDER: &str = "assets/textures/bridge";

/// Folder below `assets/textures/terrain/` with the snow's scan.
pub const SNOW_FOLDER: &str = "snow";

/// The snow's textures from `assets/textures/terrain/snow/`, as one-layer
/// arrays: colour with roughness in alpha (sRGB colour), and the normal
/// with occlusion in alpha. Nothing when there is no colour map.
pub fn load_snow() -> Option<[Texture; 2]> {
    let layer = load_layer(&Path::new(FOLDER).join(SNOW_FOLDER), true)?;
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

/// The textures in `folder`, if it has a colour map; `needed`: say so when
/// it has none.
fn load_layer(folder: &Path, needed: bool) -> Option<LayerImages> {
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
    fn stand_ins_come_first() {
        for (index, layer) in LAYERS.iter().enumerate() {
            assert!(layer.stand_in.is_none_or(|s| s < index), "{}", layer.folder);
        }
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
