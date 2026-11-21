use cgmath::{Matrix4, Point3, Vector3};

use crate::core::{
    assets::{self, AssetKind, AssetRef},
    material::asset as material_asset,
    entity::Entity,
    model::{Model, ModelBuilder},
    renderer::{pbr::Lighting, weather::exposure},
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
    /// The asset database's state the materials were applied at
    /// (`material::asset::generation`): a saved material shows at once.
    materials_at: u64,
}

pub struct ModelComponent {
    model: Model,
    /// Material parameters set from scripts (material name, parameter,
    /// value), kept over reloads of the asset's materials. Not saved.
    param_overrides: Vec<(String, String, [f32; 4])>,
    /// `None` for a model that was built in code from a raw file.
    source: Option<Source>,
    /// Counts the times the model was replaced (`generation`).
    generation: u64,
}

impl ModelComponent {
    pub fn new(model: Model) -> Self {
        ModelComponent {
            model,
            param_overrides: Vec::new(),
            source: None,
            generation: 0,
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
            param_overrides: Vec::new(),
            source: Some(Source {
                asset: asset.clone(),
                base_scale,
                revision,
                materials_at: material_asset::generation(),
            }),
            generation: 0,
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

    /// Changes whenever another model is loaded in place of this one (a
    /// re-imported asset, another asset): what was taken from the old
    /// one's skeleton must be taken again.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Set a parameter of the materials named `material` (all of them for
    /// `""`) for this entity alone, e.g. a lamp's glow from a script:
    /// `value` holds a float in its first number, a vector or colour
    /// (linear) in the first two to four. In memory only: the scene file
    /// keeps the asset's material. Returns false when no material of that
    /// name has such a parameter.
    pub fn set_material_param(&mut self, material: &str, param: &str, value: [f32; 4]) -> bool {
        self.param_overrides.retain(|(m, p, _)| !(m == material && p == param));
        self.param_overrides.push((material.to_string(), param.to_string(), value));
        self.apply_param_overrides()
    }

    /// Back to the asset's values for all parameters set by
    /// `set_material_param`.
    pub fn clear_material_params(&mut self) {
        self.param_overrides.clear();
        self.reapply_materials();
    }

    /// The parameters scripts set, on the model's materials. False if one
    /// fit no material.
    fn apply_param_overrides(&mut self) -> bool {
        let mut all = true;
        for (material, param, value) in &self.param_overrides {
            let mut any = false;
            for index in 0..self.model.materials().len() {
                let Some(m) = self.model.material_mut(index) else { continue };
                if material.is_empty() || m.name == *material {
                    any |= m.set_number(param, *value);
                }
            }
            all &= any;
        }
        all
    }

    /// The asset's materials again (and the scripts' parameters on them).
    fn reapply_materials(&mut self) {
        let slots = self.source.as_ref().and_then(|source| {
            assets::database().get(source.asset.id).and_then(|e| e.meta.as_model()).map(|m| m.materials.clone())
        });
        match slots {
            Some(slots) => material_asset::apply_to_model(&mut self.model, &slots),
            None => self.model.reset_materials(),
        }
        self.apply_param_overrides();
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
        self.generation += 1;
        self.source = Some(Source {
            asset,
            base_scale,
            revision,
            materials_at: material_asset::generation(),
        });
        Ok(())
    }

    /// Load the model again if its asset was saved since.
    fn reload_if_outdated(&mut self) {
        let Some(source) = &self.source else {
            return;
        };
        let (current, slots, generation) = {
            let database = assets::database();
            let entry = database.get(source.asset.id);
            (
                entry.map(|entry| entry.revision),
                entry.and_then(|e| e.meta.as_model()).map(|m| m.materials.clone()),
                database.changes(),
            )
        };
        if current.is_none() {
            return;
        }
        if current == Some(source.revision) {
            // Only another asset changed: maybe one of the materials.
            if generation != source.materials_at {
                if let Some(slots) = slots {
                    material_asset::apply_to_model(&mut self.model, &slots);
                    self.apply_param_overrides();
                }
                if let Some(source) = &mut self.source {
                    source.materials_at = generation;
                }
            }
            return;
        }
        match load_model(&source.asset, source.base_scale) {
            Ok((model, revision)) => {
                self.model = model;
                self.generation += 1;
                self.apply_param_overrides();
                if let Some(source) = &mut self.source {
                    source.revision = revision;
                    source.materials_at = material_asset::generation();
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
    let (source, scale, slots, revision) = {
        let database = assets::database();
        let entry = database
            .get(asset.id)
            .ok_or_else(|| format!("asset {} does not exist", asset.id))?;
        let AssetKind::Model(settings) = &entry.meta.kind else {
            return Err(format!("'{}' is not a model", entry.meta.name));
        };
        (
            entry.source_path(),
            settings.scale,
            settings.materials.clone(),
            entry.revision,
        )
    };
    let path = source.to_string_lossy().replace('\\', "/");
    let mut model = ModelBuilder::from_file(&path)
        .map_err(|e| e.to_string())?
        .with_scale(base_scale * scale)
        .build();
    model.init();
    // The asset's materials replace what the loader made of the file's.
    material_asset::apply_to_model(&mut model, &slots);
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
        // Characters keep no dry patch under them from rain or snow.
        if exposure::in_exposure_pass() && self.model.skeleton().is_some() {
            return;
        }
        // The ground map holds the terrain alone.
        if exposure::in_ground_pass() {
            return;
        }
        if let Some(lighting) = Lighting::from_scene(scene) {
            self.model.render(&lighting, parent_transform, view_projection);
        }
    }

    fn handle_event(&mut self, _: &mut glfw::Glfw, _: &mut glfw::Window, _: &glfw::WindowEvent) {}

    fn raycast(
        &self,
        transform: &Matrix4<f32>,
        origin: Point3<f32>,
        dir: Vector3<f32>,
    ) -> Option<f32> {
        self.model.raycast(transform, origin, dir)
    }

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
