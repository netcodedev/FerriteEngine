//! A model's materials and the textures they are given.

use russimp_ng::material::TextureType;

use std::collections::HashMap;

use crate::core::{
    assets::MaterialMap,
    material::{srgb8_to_linear, standard::params, AlphaMode, Material},
    renderer::texture::Texture,
};

use super::{
    material_maps::{self, MapKind},
    Model,
};

impl Model {
    /// Display name of the material at `idx`, if assimp reported one.
    pub fn material_name_at(&self, idx: usize) -> Option<String> {
        self.model
            .materials
            .get(idx)
            .and_then(|m| Self::material_name(m))
    }

    /// The material at a material index.
    pub fn material(&self, idx: usize) -> Option<&Material> {
        self.materials.get(idx)
    }

    pub fn material_mut(&mut self, idx: usize) -> Option<&mut Material> {
        self.materials.get_mut(idx)
    }

    /// Draw the parts of material `idx` with `material` instead.
    pub fn set_material(&mut self, idx: usize, material: Material) {
        if let Some(slot) = self.materials.get_mut(idx) {
            *slot = material;
        }
    }

    pub fn materials(&self) -> &[Material] {
        &self.materials
    }

    /// What the loader made of material `idx` of the file.
    pub fn file_material(&self, idx: usize) -> Option<&Material> {
        self.file_materials.get(idx)
    }

    /// Back to what the loader made of the file's materials.
    pub fn reset_materials(&mut self) {
        self.materials = self.file_materials.clone();
    }

    /// Replace the texture a material uses in one slot of Standard PBR,
    /// e.g. with one the user assigned to the asset.
    pub fn set_material_texture(
        &mut self,
        material_index: usize,
        slot: TextureType,
        texture: Texture,
    ) {
        if let Some(material) = self.materials.get_mut(material_index) {
            set_standard_map(material, &slot, texture);
        }
    }

    /// Colour a material plainly: `color` is sRGB, each channel 0 to 1.
    /// For models made in code (see `generated`), whose materials have no
    /// maps of their own.
    pub fn set_material_color(&mut self, material_index: usize, color: [f32; 3]) {
        if let Some(material) = self.materials.get_mut(material_index) {
            set_standard_color(material, color);
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

    /// Standard PBR materials of the maps `init` found, one per material
    /// of the file. glTF materials without any map get their constant
    /// factors: without, such a material is drawn plain white, rough and
    /// not metallic. Materials with maps (a textured tree) keep the maps
    /// alone, so they look as before.
    pub(super) fn materials_from_maps(&self, all_maps: Vec<HashMap<TextureType, Texture>>) -> Vec<Material> {
        all_maps
            .into_iter()
            .enumerate()
            .map(|(index, maps)| {
                let factors = (maps.is_empty())
                    .then(|| self.model.materials.get(index).and_then(gltf_factors))
                    .flatten();
                let mut material = material_from_maps(maps);
                material.name = self.material_name_at(index).unwrap_or_default();
                material.foliage = super::outline::looks_like_foliage(&material.name);
                if let Some(factors) = factors {
                    let [r, g, b, _] = factors.base_color;
                    set_standard_color(&mut material, [r, g, b].map(linear_to_srgb));
                    // As 8-bit maps read them.
                    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() / 255.0;
                    material.set_float(params::ROUGHNESS, q(factors.roughness));
                    material.set_float(params::METALLIC, q(factors.metallic));
                }
                material
            })
            .collect()
    }
}

/// Standard PBR with the maps of a file's material. A colour map may cut
/// out what its alpha says (foliage cards), as every model's did before
/// materials.
pub fn material_from_maps(mut maps: HashMap<TextureType, Texture>) -> Material {
    let mut material = Material::standard();
    // A material may tag its colour map oddly; any map that is not one of
    // the data maps is taken for it.
    let base = [TextureType::Diffuse, TextureType::BaseColor]
        .iter()
        .find_map(|t| maps.remove(t))
        .or_else(|| {
            let key = maps.keys().find(|t| !is_data_map(t)).cloned()?;
            maps.remove(&key)
        });
    if let Some(base) = base {
        set_standard_map(&mut material, &TextureType::Diffuse, base);
    }
    for (slot, texture) in maps {
        set_standard_map(&mut material, &slot, texture);
    }
    material
}

/// The Standard PBR parameter a map of this assimp type fills.
pub fn standard_map_param(slot: &TextureType) -> Option<&'static str> {
    match slot {
        TextureType::Diffuse | TextureType::BaseColor => Some(params::BASE_COLOR_MAP),
        TextureType::Normals | TextureType::NormalCamera => Some(params::NORMAL_MAP),
        TextureType::Roughness => Some(params::ROUGHNESS_MAP),
        TextureType::Metalness => Some(params::METALLIC_MAP),
        TextureType::AmbientOcclusion => Some(params::OCCLUSION_MAP),
        _ => None,
    }
}

fn set_standard_map(material: &mut Material, slot: &TextureType, texture: Texture) {
    let Some(param) = standard_map_param(slot) else { return };
    if param == params::BASE_COLOR_MAP && material.alpha == AlphaMode::Opaque {
        material.alpha = AlphaMode::Cutout(0.5);
    }
    material.set_texture(param, Some(texture));
}

/// A plain sRGB colour instead of the colour map.
fn set_standard_color(material: &mut Material, color: [f32; 3]) {
    let [r, g, b] = color.map(srgb8_to_linear);
    material.set_texture(params::BASE_COLOR_MAP, None);
    material.set_number(params::BASE_COLOR, [r, g, b, 1.0]);
    material.alpha = AlphaMode::Opaque;
}

fn is_data_map(texture_type: &TextureType) -> bool {
    matches!(
        texture_type,
        TextureType::Normals
            | TextureType::NormalCamera
            | TextureType::Height
            | TextureType::Displacement
            | TextureType::Roughness
            | TextureType::Metalness
            | TextureType::AmbientOcclusion
            | TextureType::Shininess
            | TextureType::Specular
            | TextureType::Opacity
    )
}

/// A glTF material's constant factors (its PBR "metallic-roughness"
/// values without maps), as assimp reports them. Colours are linear.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GltfFactors {
    pub base_color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
}

/// The factors of a material read from a glTF file; `None` for materials
/// of other formats (FBX, OBJ), whose colour values are left alone so
/// those models look as they did.
pub(crate) fn gltf_factors(material: &russimp_ng::material::Material) -> Option<GltfFactors> {
    use russimp_ng::material::PropertyTypeInfo;
    let floats = |key: &str| {
        material.properties.iter().find_map(|p| match (&p.data, p.key == key) {
            (PropertyTypeInfo::FloatArray(values), true) => Some(values.clone()),
            _ => None,
        })
    };
    let is_gltf = material.properties.iter().any(|p| p.key.starts_with("$mat.gltf."));
    if !is_gltf {
        return None;
    }
    let base = floats("$clr.base").unwrap_or_else(|| vec![1.0; 4]);
    let get = |i: usize| base.get(i).copied().unwrap_or(1.0);
    Some(GltfFactors {
        base_color: [get(0), get(1), get(2), get(3)],
        metallic: floats("$mat.metallicFactor").and_then(|v| v.first().copied()).unwrap_or(1.0),
        roughness: floats("$mat.roughnessFactor").and_then(|v| v.first().copied()).unwrap_or(1.0),
    })
}

/// Linear colour to sRGB, each channel 0 to 1.
pub fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
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

thread_local! {
    /// Image files decoded ahead (`predecode`), taken by `read_image`.
    static PREDECODED: std::cell::RefCell<HashMap<std::path::PathBuf, crate::core::renderer::texture::decode::Decoded>> =
        std::cell::RefCell::new(HashMap::new());
}

/// Decode these image files on all cores now; `read_image` takes them from
/// here. `forget_predecoded` drops what was not taken.
pub(super) fn predecode(paths: &[std::path::PathBuf]) {
    if paths.is_empty() {
        return;
    }
    let decoded = crate::core::renderer::texture::decode::decode_files(paths);
    PREDECODED.with(|p| p.borrow_mut().extend(decoded));
}

pub(super) fn forget_predecoded() {
    PREDECODED.with(|p| p.borrow_mut().clear());
}

pub(super) fn read_image(path: &std::path::Path) -> Option<image::RgbaImage> {
    let decoded = PREDECODED
        .with(|p| p.borrow_mut().remove(path))
        .unwrap_or_else(|| {
            if !path.is_file() {
                return Err("no such file".into());
            }
            crate::core::renderer::texture::decode::decode_file(path)
        });
    match decoded {
        Ok(image) => Some(image),
        Err(e) => {
            if path.is_file() {
                log::warn!("model: could not read texture {}: {e}", path.display());
            }
            None
        }
    }
}

/// The image files `maps_by_name` will read for the maps in `wanted`:
/// those maps, and the opacity map when the colour map is among them.
pub(super) fn files_by_name(
    material_name: Option<&str>,
    model_stem: &str,
    model_dir: &str,
    wanted: &[MaterialMap],
) -> Vec<std::path::PathBuf> {
    let prefixes = material_maps::name_prefixes(material_name, model_stem);
    let files = material_maps::images_with_prefix(std::path::Path::new(model_dir), &prefixes);
    material_maps::find_maps(&files, &prefixes)
        .into_iter()
        .filter(|(kind, _)| match kind.slot() {
            Some(map) => wanted.contains(&map),
            None => *kind == MapKind::Opacity && wanted.contains(&MaterialMap::BaseColor),
        })
        .map(|(_, path)| path)
        .collect()
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

#[cfg(test)]
mod tests {
    use russimp_ng::scene::{PostProcess, Scene};

    use super::{gltf_factors, linear_to_srgb};
    use crate::core::assets::{database::tests::TempDir, test_models};

    #[test]
    fn a_gltf_material_without_maps_gives_its_colour_factors() {
        let temp = TempDir::new("gltf-factors");
        let file = temp.0.join("box.gltf");
        test_models::write_gltf(&file, &[("box", &[])], None);
        let scene = Scene::from_file(file.to_str().unwrap(), vec![PostProcess::Triangulate]).unwrap();
        let factors = gltf_factors(&scene.materials[0]).expect("a glTF material");
        assert_eq!(factors.base_color, [0.2, 0.5, 0.2, 1.0]);
        // Not in the file: glTF's defaults.
        assert_eq!((factors.metallic, factors.roughness), (1.0, 1.0));
    }

    #[test]
    fn other_formats_keep_their_colours_out_of_it() {
        let temp = TempDir::new("obj-factors");
        let obj = temp.0.join("box.obj");
        std::fs::write(temp.0.join("box.mtl"), "newmtl red\nKd 0.8 0.1 0.1\n").unwrap();
        std::fs::write(&obj, "mtllib box.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nusemtl red\nf 1 2 3\n").unwrap();
        let scene = Scene::from_file(obj.to_str().unwrap(), vec![PostProcess::Triangulate]).unwrap();
        assert!(scene.materials.iter().all(|m| gltf_factors(m).is_none()));
    }

    #[test]
    fn linear_colours_are_turned_into_srgb() {
        assert_eq!(linear_to_srgb(0.0), 0.0);
        assert!((linear_to_srgb(1.0) - 1.0).abs() < 1e-6);
        assert!((linear_to_srgb(0.214) - 0.5).abs() < 0.01);
    }
}
