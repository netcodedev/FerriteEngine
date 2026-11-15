use russimp_ng::material::TextureType;

use crate::core::{assets::MaterialMap, renderer::texture::Texture};

use super::{material_maps::{self, MapKind}, Model};

impl Model {
    /// Display name of the material at `idx`, if assimp reported one.
    pub fn material_name_at(&self, idx: usize) -> Option<String> {
        self.model
            .materials
            .get(idx)
            .and_then(|m| Self::material_name(m))
    }

    /// Texture set for a given material index, as collected by `init`.
    pub fn material_textures(
        &self,
        idx: usize,
    ) -> Option<&std::collections::HashMap<russimp_ng::material::TextureType, crate::core::renderer::texture::Texture>> {
        self.textures.get(idx)
    }

    /// Replace the texture a material uses in one slot, e.g. with one the
    /// user assigned to the asset.
    pub fn set_material_texture(
        &mut self,
        material_index: usize,
        slot: TextureType,
        texture: Texture,
    ) {
        if let Some(textures) = self.textures.get_mut(material_index) {
            textures.insert(slot, texture);
        }
    }

    /// Use the image file at `path` as one map of a material, e.g. one the
    /// user assigned to the asset. Returns false if it cannot be read.
    pub fn set_material_map_file(
        &mut self,
        material_index: usize,
        map: MaterialMap,
        path: &std::path::Path,
    ) -> bool {
        let Some(image) = read_image(path) else {
            return false;
        };
        let slot = map_texture_type(map);
        let texture = upload_map(image, &slot);
        self.set_material_texture(material_index, slot, texture);
        true
    }

    /// Number of materials in the file.
    pub fn material_count(&self) -> usize {
        self.model.materials.len()
    }
}

/// The slot a texture of this assimp type is kept in. Spellings of the same
/// map share one slot.
pub(super) fn canonical_slot(texture_type: &TextureType) -> TextureType {
    match texture_type {
        TextureType::BaseColor => TextureType::Diffuse,
        TextureType::NormalCamera => TextureType::Normals,
        other => other.clone(),
    }
}

/// The slot a material map is kept in.
pub(super) fn map_texture_type(map: MaterialMap) -> TextureType {
    match map {
        MaterialMap::BaseColor => TextureType::Diffuse,
        MaterialMap::Normal => TextureType::Normals,
        MaterialMap::Roughness => TextureType::Roughness,
        MaterialMap::Metallic => TextureType::Metalness,
        MaterialMap::Occlusion => TextureType::AmbientOcclusion,
    }
}

pub(super) fn read_image(path: &std::path::Path) -> Option<image::RgbaImage> {
    let bytes = std::fs::read(path).ok()?;
    match image::load_from_memory(&bytes) {
        Ok(image) => Some(image.to_rgba8()),
        Err(e) => {
            log::warn!("model: could not read texture {}: {e}", path.display());
            None
        }
    }
}

/// Upload an image as the map in `slot`; only the colour map is sRGB.
pub(super) fn upload_map(image: image::RgbaImage, slot: &TextureType) -> Texture {
    let texture = Texture::new();
    let (width, height) = image.dimensions();
    texture.load_material_map(width, height, image.into_raw(), *slot == TextureType::Diffuse);
    texture
}

/// The maps in `wanted` found next to the model by file name (see
/// `material_maps`). A separate opacity map goes into the colour map's
/// alpha; a gloss map is inverted into roughness.
pub(super) fn maps_by_name(
    material_name: Option<&str>,
    model_stem: &str,
    model_dir: &str,
    wanted: &[MaterialMap],
) -> Vec<(MaterialMap, Texture)> {
    let prefixes = material_maps::name_prefixes(material_name, model_stem);
    let files = material_maps::images_with_prefix(std::path::Path::new(model_dir), &prefixes);
    let found = material_maps::find_maps(&files, &prefixes);
    let opacity = found
        .iter()
        .find(|(kind, _)| *kind == MapKind::Opacity)
        .and_then(|(_, path)| read_image(path));

    let mut maps = Vec::new();
    for (kind, path) in &found {
        let Some(map) = kind.slot().filter(|map| wanted.contains(map)) else {
            continue;
        };
        let Some(mut image) = read_image(path) else {
            continue;
        };
        if *kind == MapKind::Gloss {
            for pixel in image.pixels_mut() {
                for channel in &mut pixel.0[..3] {
                    *channel = 255 - *channel;
                }
            }
        }
        if map == MaterialMap::BaseColor {
            if let Some(opacity) = &opacity {
                if opacity.dimensions() == image.dimensions() {
                    for (pixel, alpha) in image.pixels_mut().zip(opacity.pixels()) {
                        pixel.0[3] = alpha.0[0];
                    }
                } else {
                    log::warn!(
                        "model: opacity map size {:?} differs from {} {:?}, not used",
                        opacity.dimensions(),
                        path.display(),
                        image.dimensions()
                    );
                }
            }
        }
        maps.push((map, upload_map(image, &map_texture_type(map))));
    }
    maps
}

/// Where a texture a material names (`raw_path`) may be, best first: as a
/// path relative to the model (as OBJ materials write them, e.g.
/// `../shared/textures/wood.jpg`), then by its file name (spaces and
/// underscores either way) next to the model, in its `textures` folder and
/// the project's model and texture folders, and last as written.
pub(super) fn texture_candidates(raw_path: &str, model_dir: &str) -> Vec<String> {
    let trimmed = raw_path.trim_end_matches('\0').trim();
    let Some(file_name) = super::outline::texture_file_name(raw_path) else {
        return Vec::new();
    };
    // Some FBX exporters write spaces in texture names while the on-disk
    // file uses underscores (or vice versa), so try both spellings.
    let mut name_variants = vec![file_name.to_string()];
    for variant in [file_name.replace(' ', "_"), file_name.replace('_', " ")] {
        if !name_variants.contains(&variant) {
            name_variants.push(variant);
        }
    }
    let dirs = [
        model_dir.to_string(),
        format!("{model_dir}/textures"),
        "assets/models".to_string(),
        "assets/models/textures".to_string(),
        "assets/textures".to_string(),
    ];
    let mut candidates: Vec<String> = Vec::new();
    // Absolute here or on Windows (`C:\...`): from another machine.
    let absolute = std::path::Path::new(trimmed).is_absolute()
        || trimmed.starts_with('\\')
        || trimmed.as_bytes().get(1) == Some(&b':');
    if !trimmed.is_empty() && !absolute {
        candidates.push(format!("{model_dir}/{}", trimmed.replace('\\', "/")));
    }
    for dir in &dirs {
        for name in &name_variants {
            candidates.push(format!("{dir}/{name}"));
        }
    }
    candidates.push(trimmed.to_string());
    candidates
}
