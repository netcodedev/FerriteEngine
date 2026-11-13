use cgmath::Matrix4;
use russimp_ng::material::TextureType;

use crate::core::{
    assets::{self, AssetKind, AssetRef},
    entity::Entity,
    model::{Model, ModelBuilder},
    renderer::light::skylight,
    scene::{scene_file::ComponentData, Scene},
};

use super::{Component, Property};

/// The asset a component's model was loaded from.
struct Source {
    asset: AssetRef,
    /// Scale the user of the component applies on top of the asset's own.
    base_scale: f32,
    /// Revision of the asset the model was loaded at.
    revision: u64,
}

pub struct ModelComponent {
    model: Model,
    /// `None` for a model that was built in code from a raw file.
    source: Option<Source>,
}

impl ModelComponent {
    pub fn new(model: Model) -> Self {
        ModelComponent {
            model,
            source: None,
        }
    }

    /// The model of an asset: the variant the reference names, or the first
    /// one without, with the asset's scale and texture overrides applied.
    /// Must run on the GL thread.
    pub fn from_asset(asset: &AssetRef) -> Result<Self, String> {
        Self::from_asset_scaled(asset, 1.0)
    }

    /// Like `from_asset`, with `base_scale` applied on top of the asset's
    /// scale (for users with a unit convention of their own).
    pub fn from_asset_scaled(asset: &AssetRef, base_scale: f32) -> Result<Self, String> {
        let (model, revision) = load_model(asset, base_scale)?;
        Ok(ModelComponent {
            model,
            source: Some(Source {
                asset: asset.clone(),
                base_scale,
                revision,
            }),
        })
    }

    pub fn get_model(&self) -> &Model {
        &self.model
    }

    pub fn get_model_mut(&mut self) -> &mut Model {
        &mut self.model
    }

    pub fn into_model(self) -> Model {
        self.model
    }

    /// The asset the model comes from, if it comes from one.
    pub fn asset(&self) -> Option<&AssetRef> {
        self.source.as_ref().map(|source| &source.asset)
    }

    /// Show another asset. On failure the current model stays.
    pub fn set_asset(&mut self, asset: AssetRef) -> Result<(), String> {
        let base_scale = self.source.as_ref().map_or(1.0, |source| source.base_scale);
        let (model, revision) = load_model(&asset, base_scale)?;
        self.model = model;
        self.source = Some(Source {
            asset,
            base_scale,
            revision,
        });
        Ok(())
    }

    /// Load the model again if its asset was saved since.
    fn reload_if_outdated(&mut self) {
        let Some(source) = &self.source else {
            return;
        };
        let current = assets::database()
            .get(source.asset.id)
            .map(|entry| entry.revision);
        if current.is_none() || current == Some(source.revision) {
            return;
        }
        match load_model(&source.asset, source.base_scale) {
            Ok((model, revision)) => {
                self.model = model;
                if let Some(source) = &mut self.source {
                    source.revision = revision;
                }
            }
            Err(e) => {
                log::warn!("model: could not reload asset {}: {e}", source.asset.id);
                // Do not try again every frame.
                if let (Some(source), Some(revision)) = (&mut self.source, current) {
                    source.revision = revision;
                }
            }
        }
    }
}

fn load_model(asset: &AssetRef, base_scale: f32) -> Result<(Model, u64), String> {
    // Take what is needed out of the database and let go of it before the
    // (slow) load.
    let (source, scale, overrides, revision) = {
        let database = assets::database();
        let entry = database
            .get(asset.id)
            .ok_or_else(|| format!("asset {} does not exist", asset.id))?;
        let AssetKind::Model(settings) = &entry.meta.kind else {
            return Err(format!("'{}' is not a model", entry.meta.name));
        };
        let path = |id: Option<assets::AssetId>| {
            id.and_then(|id| database.get(id))
                .map(|texture| texture.source_path())
        };
        let overrides: Vec<_> = settings
            .materials
            .iter()
            .map(|material| (path(material.base_color), path(material.normal)))
            .collect();
        (
            entry.source_path(),
            settings.scale,
            overrides,
            entry.revision,
        )
    };
    let path = source.to_string_lossy().replace('\\', "/");
    let mut model = ModelBuilder::from_file(&path)
        .map_err(|e| e.to_string())?
        .with_scale(base_scale * scale)
        .build();
    model.init();
    for (material, (base_color, normal)) in overrides.into_iter().enumerate() {
        for (file, slot) in [
            (base_color, TextureType::Diffuse),
            (normal, TextureType::Normals),
        ] {
            let Some(file) = file else {
                continue;
            };
            match Model::load_texture_file(&file) {
                Some(texture) => model.set_material_texture(material, slot, texture),
                None => log::warn!("model: could not load texture {}", file.display()),
            }
        }
    }
    model.show_variant(asset.part.as_deref())?;
    Ok((model, revision))
}

impl Component for ModelComponent {
    fn update(&mut self, _: &mut Scene, _: &mut Entity, _: f64) {
        self.reload_if_outdated();
    }

    fn update_in_editor(&mut self, _: &mut Scene, _: &mut Entity, _: f64) {
        self.reload_if_outdated();
    }

    fn render(
        &self,
        scene: &Scene,
        _: &Entity,
        view_projection: &Matrix4<f32>,
        parent_transform: &Matrix4<f32>,
    ) {
        if let Some(skylight) = scene.get_component::<skylight::SkyLight>() {
            self.model
                .render(&skylight.get_position(), &parent_transform, view_projection);
        }
    }

    fn handle_event(&mut self, _: &mut glfw::Glfw, _: &mut glfw::Window, _: &glfw::WindowEvent) {}

    fn display_name(&self) -> String {
        "Model".into()
    }

    /// Saved when the model comes from an asset. A model built in code
    /// belongs to whatever built it (e.g. the player prefab).
    fn to_data(&self) -> Option<ComponentData> {
        self.asset().map(|asset| ComponentData::Model {
            asset: asset.clone(),
        })
    }

    fn properties(&self) -> Vec<Property> {
        let label = match self.asset() {
            Some(asset) => assets::database()
                .display_name(asset)
                .unwrap_or_else(|| format!("missing asset {}", asset.id)),
            None => "built in".to_string(),
        };
        // Chosen from the assets, not typed.
        vec![Property::text("model", label).read_only()]
    }
}
