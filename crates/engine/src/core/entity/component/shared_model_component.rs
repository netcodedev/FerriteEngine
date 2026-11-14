//! A model asset drawn by many entities from one copy: loaded (meshes and
//! textures, on disk and on the GPU) the first time it is asked for, then
//! shared. For many static instances of the same thing (built pieces,
//! props) — adding one costs nothing. A model that is animated, tinted or
//! changed per entity wants a `ModelComponent` of its own.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use cgmath::{Matrix4, Point3, Vector3};

use crate::core::{
    assets::AssetRef, entity::Entity, model::Model, renderer::pbr::Lighting, scene::Scene,
};

use super::{model_component::ModelComponent, Component};

thread_local! {
    /// The models in use, by asset and asset revision. Held weakly: a model
    /// no entity shows any more is freed.
    static SHARED: RefCell<HashMap<(AssetRef, u64), std::rc::Weak<Model>>> =
        RefCell::new(HashMap::new());
}

pub struct SharedModel {
    asset: AssetRef,
    model: Rc<Model>,
}

impl SharedModel {
    /// The model of `asset` (as `ModelComponent::from_asset` loads it),
    /// loaded the first time and shared after that. Must run on the GL
    /// thread.
    pub fn from_asset(asset: &AssetRef) -> Result<Self, String> {
        let revision = crate::core::assets::database()
            .get(asset.id)
            .map_or(0, |entry| entry.revision);
        let key = (asset.clone(), revision);
        let shared = SHARED.with(|shared| shared.borrow().get(&key).and_then(|weak| weak.upgrade()));
        let model = match shared {
            Some(model) => model,
            None => {
                let model = Rc::new(ModelComponent::from_asset(asset)?.into_model());
                SHARED.with(|shared| {
                    let mut shared = shared.borrow_mut();
                    shared.retain(|_, weak| weak.strong_count() > 0);
                    shared.insert(key, Rc::downgrade(&model));
                });
                model
            }
        };
        Ok(Self {
            asset: asset.clone(),
            model,
        })
    }

    pub fn asset(&self) -> &AssetRef {
        &self.asset
    }

    pub fn get_model(&self) -> &Model {
        &self.model
    }
}

impl Component for SharedModel {
    fn update(&mut self, _: &mut Scene, _: &mut Entity, _: f64) {}

    fn render(
        &self,
        scene: &Scene,
        _: &Entity,
        view_projection: &Matrix4<f32>,
        parent_transform: &Matrix4<f32>,
    ) {
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
        "Shared model".into()
    }
}
