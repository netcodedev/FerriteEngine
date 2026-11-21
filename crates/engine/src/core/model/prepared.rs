//! Loading a model in two steps: `PreparedModel::load` does everything that
//! needs no GPU (reading the file with assimp, finding its maps and
//! decoding them on all cores) and may run on any thread;
//! `ModelBuilder::from_prepared` then `Model::init` on the render thread
//! only upload. See docs/asset-browser.md ("Vorschaubilder ohne
//! Blockieren").

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use russimp_ng::{
    material::{DataContent, PropertyTypeInfo, TextureType},
    scene::Scene,
};

use crate::core::{
    assets::{AssetId, MaterialMap},
    renderer::texture::decode::{self, Decoded},
};

use super::{
    model_materials::{canonical_slot, files_by_name, map_texture_type, texture_candidates},
    Model, ModelBuilder,
};

/// The maps of a model's file, decoded: files by path (taken by
/// `read_image`), embedded ones by (material, slot).
pub(crate) struct DecodedMaps {
    pub(crate) files: HashMap<PathBuf, Decoded>,
    pub(crate) embedded: HashMap<(usize, TextureType), Decoded>,
}

/// A model file read and its maps decoded, ready to be uploaded on the
/// render thread (`ModelBuilder::from_prepared`).
pub struct PreparedModel {
    scene: Scene,
    name: String,
    model_dir: String,
    maps: DecodedMaps,
    /// Texture asset files the material assets of `with_material_textures`
    /// use, decoded.
    material_textures: HashMap<PathBuf, Decoded>,
}

// SAFETY: a `Scene` is plain Rust data (assimp's own copy is released when
// it is converted); its `Rc`s and `RefCell`s only point at other parts of
// the same scene. A `PreparedModel` hands out no reference into it and is
// moved as a whole, so no `Rc` is ever shared between threads. The decoded
// images are plain bytes.
unsafe impl Send for PreparedModel {}

impl PreparedModel {
    /// Read the model file at `full_path` (relative to the working folder)
    /// and decode the maps `Model::init` will need. No GPU: any thread.
    pub fn load(full_path: &str) -> Result<Self, String> {
        let (scene, name, model_dir) = Model::read_file(full_path).map_err(|e| format!("{full_path}: {e}"))?;
        // Which files the render thread has uploaded is not known here: all.
        let maps = decode_maps(&scene, &name, &model_dir, &|_| false);
        Ok(Self { scene, name, model_dir, maps, material_textures: HashMap::new() })
    }

    /// Also decode the texture assets the material assets `materials`
    /// use, for `material::asset::apply_to_model` to take.
    pub fn with_material_textures(mut self, materials: &[AssetId]) -> Self {
        let files = crate::core::material::asset::texture_files_of(materials);
        self.material_textures = decode::decode_files(&files);
        self
    }

    pub(super) fn into_parts(self) -> (Scene, String, String, DecodedMaps) {
        // Kept for the render thread to hand on (see `ModelBuilder`).
        if !self.material_textures.is_empty() {
            crate::core::material::asset::offer_textures(self.material_textures);
        }
        (self.scene, self.name, self.model_dir, self.maps)
    }
}

impl ModelBuilder {
    /// A model of a `PreparedModel`; `build` then `init` on the render
    /// thread upload what it decoded.
    pub fn from_prepared(prepared: PreparedModel) -> ModelBuilder {
        ModelBuilder { model: Model::from_prepared(prepared) }
    }
}

/// Find and decode on all cores the maps `Model::init` reads for `scene`:
/// embedded ones, external ones (the first candidate on disk) and those
/// found by name for the slots still empty.
/// `skip`: files not to decode (uploaded already).
pub(crate) fn decode_maps(scene: &Scene, name: &str, model_dir: &str, skip: &dyn Fn(&Path) -> bool) -> DecodedMaps {
    let mut files = Vec::new();
    let mut embedded: Vec<((usize, TextureType), Vec<u8>)> = Vec::new();
    for (index, material) in scene.materials.iter().enumerate() {
        let mut slots: Vec<TextureType> = Vec::new();
        for (tex_type, texture) in &material.textures {
            let slot = canonical_slot(tex_type);
            if slots.contains(&slot) {
                continue;
            }
            if let DataContent::Bytes(bytes) = &texture.borrow().data {
                embedded.push(((index, slot.clone()), bytes.clone()));
                slots.push(slot);
            }
        }
        for property in &material.properties {
            if property.key != "$tex.file" {
                continue;
            }
            let slot = canonical_slot(&property.semantic);
            if slots.contains(&slot) {
                continue;
            }
            let PropertyTypeInfo::String(raw_path) = &property.data else {
                continue;
            };
            let found = texture_candidates(raw_path, model_dir).into_iter().find(|c| Path::new(c).is_file());
            if let Some(path) = found {
                files.push(PathBuf::from(path));
                slots.push(slot);
            }
        }
        let missing: Vec<MaterialMap> =
            MaterialMap::ALL.into_iter().filter(|map| !slots.contains(&map_texture_type(*map))).collect();
        if !missing.is_empty() {
            let material_name = Model::material_name(material);
            files.extend(files_by_name(material_name.as_deref(), name, model_dir, &missing));
        }
    }
    files.retain(|path| !skip(path));
    let decoded = decode::decode_all(&embedded, |(_, bytes)| decode::decode_bytes(bytes));
    DecodedMaps {
        files: decode::decode_files(&files),
        embedded: embedded.into_iter().map(|(key, _)| key).zip(decoded).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send<T: Send>() {}

    #[test]
    fn a_prepared_model_can_go_to_another_thread() {
        assert_send::<PreparedModel>();
    }

    #[test]
    fn a_missing_file_is_an_error_not_a_panic() {
        let result = std::thread::spawn(|| PreparedModel::load("no/such/model.glb").map(|_| ())).join().unwrap();
        assert!(result.is_err());
    }
}
