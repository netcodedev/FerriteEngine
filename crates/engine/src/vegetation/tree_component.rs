//! A tree grown from `TreeParams` on an entity. Every parameter is a
//! property, so the editor shows the tree change as a slider moves; its
//! bark and leaves take the project's texture assets, or textures drawn in
//! code where none is chosen. The mesh becomes a model as in
//! `core::model::generated`.

use std::path::PathBuf;

use cgmath::{Matrix4, Point3, Vector3};
use russimp_ng::material::TextureType;
use serde::{Deserialize, Serialize};

use crate::core::{
    assets::{self, AssetKind, AssetRef},
    entity::{
        component::{Component, Property, PropertyValue},
        Entity,
    },
    headless,
    model::{generated, Model},
    renderer::{pbr::Lighting, texture::Texture, weather::exposure},
    scene::{scene_file::ComponentData, Scene},
};

use super::{
    textures::{self, LeafKind},
    tree::{self, CrownShape, Foliage, TreeParams},
};

/// The textures a tree is given (texture assets); each one left out is
/// drawn in code (the normal maps are then left out).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct TreeTextures {
    pub bark: Option<AssetRef>,
    pub bark_normal: Option<AssetRef>,
    pub leaves: Option<AssetRef>,
    /// A separate cut-out for the leaves (white: leaf), for a leaf texture
    /// without alpha.
    pub leaves_opacity: Option<AssetRef>,
    pub leaves_normal: Option<AssetRef>,
}

/// The texture properties, and which field of `TreeTextures` each sets.
const TEXTURE_PROPERTIES: [&str; 5] = [
    "bark texture",
    "bark normal map",
    "leaf texture",
    "leaf opacity map",
    "leaf normal map",
];
/// The choice for no texture asset.
const NO_TEXTURE: &str = "(none)";
/// The choice of preset once a parameter differs from every preset.
const CUSTOM: &str = "custom";

impl TreeTextures {
    fn slot(&mut self, property: &str) -> Option<&mut Option<AssetRef>> {
        Some(match property {
            "bark texture" => &mut self.bark,
            "bark normal map" => &mut self.bark_normal,
            "leaf texture" => &mut self.leaves,
            "leaf opacity map" => &mut self.leaves_opacity,
            "leaf normal map" => &mut self.leaves_normal,
            _ => return None,
        })
    }
}

pub struct TreeComponent {
    params: TreeParams,
    textures: TreeTextures,
    /// Not made without a GPU (the dedicated server).
    model: Option<Model>,
    /// The parameters changed since the model was made: it is made again
    /// in the next update (once however many changed).
    dirty: bool,
    triangles: usize,
}

impl TreeComponent {
    pub fn new(params: TreeParams, textures: TreeTextures) -> Result<Self, String> {
        let mut tree = Self {
            params,
            textures,
            model: None,
            dirty: false,
            triangles: 0,
        };
        tree.rebuild()?;
        Ok(tree)
    }

    pub fn params(&self) -> &TreeParams {
        &self.params
    }

    /// The preset these parameters are (whatever the seed), if any.
    fn preset_name(&self) -> &'static str {
        TreeParams::PRESETS
            .into_iter()
            .find(|name| {
                TreeParams::preset(name).is_some_and(|p| TreeParams { seed: self.params.seed, ..p } == self.params)
            })
            .unwrap_or(CUSTOM)
    }

    fn rebuild(&mut self) -> Result<(), String> {
        self.dirty = false;
        let mesh = tree::grow(&self.params);
        self.triangles = mesh.triangle_count();
        if headless::is_headless() {
            return Ok(());
        }
        let mut model = generated::build_model(&[("bark", &mesh.bark), ("leaves", &mesh.leaves)])?;
        self.apply_textures(&mut model);
        self.model = Some(model);
        Ok(())
    }

    fn apply_textures(&self, model: &mut Model) {
        let source = |asset: &Option<AssetRef>| -> Option<PathBuf> {
            let asset = asset.as_ref()?;
            let database = assets::database();
            let entry = database.get(asset.id)?;
            matches!(entry.meta.kind, AssetKind::Texture(_)).then(|| entry.source_path())
        };
        let read = |asset: &Option<AssetRef>| -> Option<image::RgbaImage> {
            let path = source(asset)?;
            match image::open(&path) {
                Ok(image) => Some(image.to_rgba8()),
                Err(e) => {
                    log::warn!("tree: could not read {}: {e}", path.display());
                    None
                }
            }
        };
        let upload = |image: image::RgbaImage, srgb: bool| {
            let texture = Texture::new();
            let (w, h) = image.dimensions();
            texture.load_material_map(w, h, image.into_raw(), srgb);
            texture
        };
        let drawn = |image: textures::Image| {
            image::RgbaImage::from_raw(image.size, image.size, image.pixels).expect("a square RGBA image")
        };
        for material in 0..model.material_count() {
            let name = model.material_name_at(material).unwrap_or_default();
            let (color, normal) = if name.contains("leaves") {
                let mut color = read(&self.textures.leaves).unwrap_or_else(|| {
                    drawn(textures::leaves(LeafKind::for_tree(self.params.foliage, self.params.crown)))
                });
                // A separate cut-out into the colour's alpha.
                if let Some(opacity) = read(&self.textures.leaves_opacity) {
                    let opacity = image::imageops::resize(
                        &opacity,
                        color.width(),
                        color.height(),
                        image::imageops::FilterType::Triangle,
                    );
                    for (c, o) in color.pixels_mut().zip(opacity.pixels()) {
                        c[3] = o[0];
                    }
                }
                (color, read(&self.textures.leaves_normal))
            } else if name.contains("bark") {
                (
                    read(&self.textures.bark).unwrap_or_else(|| drawn(textures::bark())),
                    read(&self.textures.bark_normal),
                )
            } else {
                continue;
            };
            model.set_material_texture(material, TextureType::Diffuse, upload(color, true));
            if let Some(normal) = normal {
                model.set_material_texture(material, TextureType::Normals, upload(normal, false));
            }
        }
    }

    /// The texture assets of the project, by name (sorted), for a choice.
    fn texture_names() -> Vec<String> {
        let database = assets::database();
        let mut names: Vec<String> = database
            .iter()
            .filter(|entry| matches!(entry.meta.kind, AssetKind::Texture(_)))
            .map(|entry| entry.meta.name.clone())
            .collect();
        names.sort();
        names.dedup();
        names
    }

    fn texture_name(asset: &Option<AssetRef>) -> String {
        asset
            .as_ref()
            .and_then(|a| assets::database().get(a.id).map(|e| e.meta.name.clone()))
            .unwrap_or_else(|| NO_TEXTURE.to_string())
    }

    fn texture_by_name(name: &str) -> Option<AssetRef> {
        let database = assets::database();
        let found = database
            .iter()
            .find(|entry| matches!(entry.meta.kind, AssetKind::Texture(_)) && entry.meta.name == name)
            .map(|entry| AssetRef::whole(entry.meta.id));
        found
    }

    fn update_model(&mut self) {
        if self.dirty {
            if let Err(e) = self.rebuild() {
                log::warn!("tree: {e}");
            }
        }
    }
}

impl Component for TreeComponent {
    fn update(&mut self, _: &mut Scene, _: &mut Entity, _: f64) {
        self.update_model();
    }

    fn update_in_editor(&mut self, _: &mut Scene, _: &mut Entity, _: f64) {
        self.update_model();
    }

    fn render(&self, scene: &Scene, _: &Entity, view_projection: &Matrix4<f32>, parent_transform: &Matrix4<f32>) {
        // The ground map holds the terrain alone.
        if exposure::in_ground_pass() {
            return;
        }
        if let (Some(model), Some(lighting)) = (&self.model, Lighting::from_scene(scene)) {
            model.render(&lighting, parent_transform, view_projection);
        }
    }

    fn handle_event(&mut self, _: &mut glfw::Glfw, _: &mut glfw::Window, _: &glfw::WindowEvent) {}

    fn raycast(&self, transform: &Matrix4<f32>, origin: Point3<f32>, dir: Vector3<f32>) -> Option<f32> {
        self.model.as_ref()?.raycast(transform, origin, dir)
    }

    fn display_name(&self) -> String {
        "Tree".into()
    }

    fn to_data(&self) -> Option<ComponentData> {
        Some(ComponentData::Tree {
            params: self.params.clone(),
            textures: self.textures.clone(),
        })
    }

    fn properties(&self) -> Vec<Property> {
        let p = &self.params;
        let mut presets: Vec<String> = vec![CUSTOM.into()];
        presets.extend(TreeParams::PRESETS.iter().map(|s| s.to_string()));
        let mut properties = vec![
            Property::choice("preset", self.preset_name().to_string(), presets),
            Property::int("seed", p.seed as i64),
            Property::ranged("height", p.height, 0.2, 40.0),
            Property::ranged("trunk radius", p.trunk_radius, 0.02, 2.0),
            Property::ranged("taper", p.taper, 0.0, 1.0),
            Property::ranged("flare", p.flare, 0.0, 1.5),
            Property::ranged("lean", p.lean, -40.0, 40.0),
            Property::ranged("bend", p.bend, 0.0, 1.0),
            Property::ranged("levels", p.levels as f32, 0.0, tree::MAX_LEVELS as f32),
            Property::ranged("branches", p.branches as f32, 0.0, 120.0),
            Property::ranged("twigs", p.twigs as f32, 0.0, 20.0),
            Property::ranged("branch start", p.branch_start, 0.0, 1.0),
            Property::ranged("branch angle", p.branch_angle, 0.0, 150.0),
            Property::ranged("branch length", p.branch_length, 0.0, 2.0),
            Property::ranged("twig length", p.twig_length, 0.0, 1.5),
            Property::choice(
                "crown",
                p.crown.name().to_string(),
                CrownShape::ALL.iter().map(|c| c.name().to_string()).collect(),
            ),
            Property::ranged("droop", p.droop, -1.0, 2.0),
            Property::ranged("branch radius", p.branch_radius, 0.1, 1.0),
            Property::choice(
                "foliage",
                p.foliage.name().to_string(),
                Foliage::ALL.iter().map(|f| f.name().to_string()).collect(),
            ),
            Property::ranged("leaf size", p.leaf_size, 0.1, 3.0),
            Property::ranged("leaf density", p.leaf_density, 0.0, 20.0),
            Property::ranged("frond width", p.frond_width, 0.1, 3.0),
            Property::ranged("bark tile", p.bark_tile, 0.2, 5.0),
        ];
        let mut choices = vec![NO_TEXTURE.to_string()];
        choices.extend(Self::texture_names());
        let textures = &self.textures;
        for (name, asset) in TEXTURE_PROPERTIES.iter().zip([
            &textures.bark,
            &textures.bark_normal,
            &textures.leaves,
            &textures.leaves_opacity,
            &textures.leaves_normal,
        ]) {
            properties.push(Property::choice(*name, Self::texture_name(asset), choices.clone()));
        }
        properties.push(Property::text("triangles", self.triangles.to_string()).read_only());
        properties
    }

    fn set_property(&mut self, name: &str, value: PropertyValue) {
        let p = &mut self.params;
        let number = value.as_f32();
        let count = number.map(|v| v.round().max(0.0) as u32);
        let text = value.as_text().map(str::to_string);
        match name {
            "preset" => {
                let Some(preset) = text.as_deref().and_then(TreeParams::preset) else {
                    return;
                };
                *p = TreeParams { seed: p.seed, ..preset };
            }
            "seed" => match value {
                PropertyValue::Int(v) => p.seed = v.max(0) as u64,
                _ => return,
            },
            "height" => p.height = number.unwrap_or(p.height),
            "trunk radius" => p.trunk_radius = number.unwrap_or(p.trunk_radius),
            "taper" => p.taper = number.unwrap_or(p.taper),
            "flare" => p.flare = number.unwrap_or(p.flare),
            "lean" => p.lean = number.unwrap_or(p.lean),
            "bend" => p.bend = number.unwrap_or(p.bend),
            "levels" => p.levels = count.unwrap_or(p.levels).min(tree::MAX_LEVELS),
            "branches" => p.branches = count.unwrap_or(p.branches),
            "twigs" => p.twigs = count.unwrap_or(p.twigs),
            "branch start" => p.branch_start = number.unwrap_or(p.branch_start),
            "branch angle" => p.branch_angle = number.unwrap_or(p.branch_angle),
            "branch length" => p.branch_length = number.unwrap_or(p.branch_length),
            "twig length" => p.twig_length = number.unwrap_or(p.twig_length),
            "crown" => p.crown = text.as_deref().and_then(CrownShape::from_name).unwrap_or(p.crown),
            "droop" => p.droop = number.unwrap_or(p.droop),
            "branch radius" => p.branch_radius = number.unwrap_or(p.branch_radius),
            "foliage" => p.foliage = text.as_deref().and_then(Foliage::from_name).unwrap_or(p.foliage),
            "leaf size" => p.leaf_size = number.unwrap_or(p.leaf_size),
            "leaf density" => p.leaf_density = number.unwrap_or(p.leaf_density),
            "frond width" => p.frond_width = number.unwrap_or(p.frond_width),
            "bark tile" => p.bark_tile = number.unwrap_or(p.bark_tile),
            _ => {
                let Some(slot) = self.textures.slot(name) else {
                    return;
                };
                let Some(text) = text else {
                    return;
                };
                *slot = if text == NO_TEXTURE { None } else { Self::texture_by_name(&text) };
            }
        }
        *p = p.sanitized();
        self.dirty = true;
    }
}
