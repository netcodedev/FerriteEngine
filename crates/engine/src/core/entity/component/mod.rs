use as_any::AsAny;

use cgmath::{Matrix4, Point3, Vector3};
use glfw::{Glfw, Window};

use crate::core::{
    physics::physics_engine::PhysicsEngine,
    scene::{scene_file::ComponentData, Scene},
};

use super::Entity;

pub use property::{Property, PropertyValue};

pub trait Component: AsAny {
    fn update(&mut self, scene: &mut Scene, entity: &mut Entity, delta_time: f64);
    /// After every entity's `update` this frame (see `Behaviour::late_update`).
    fn late_update(&mut self, _scene: &mut Scene, _entity: &mut Entity, _delta_time: f64) {}
    fn render(
        &self,
        _scene: &Scene,
        _entity: &Entity,
        _view_projection: &Matrix4<f32>,
        _parent_transform: &Matrix4<f32>,
    ) {
    }
    /// Second render phase. Runs after every entity's opaque `render` has
    /// finished, so the depth buffer already contains all opaque geometry
    /// (terrain, player, props). Use this for transparency that needs to
    /// write depth without occluding the player or other dynamic geometry.
    fn render_transparent(
        &self,
        _scene: &Scene,
        _entity: &Entity,
        _view_projection: &Matrix4<f32>,
        _parent_transform: &Matrix4<f32>,
    ) {
    }
    /// Third render phase: 2D drawing on top of the finished, tone mapped
    /// image (a HUD, an inventory, menus), with `PlaneRenderer` and `Text`
    /// in the viewport's pixels. Depth testing is off; what is drawn later
    /// lies on top.
    fn render_overlay(&self, _scene: &Scene, _entity: &Entity) {}
    fn handle_event(&mut self, glfw: &mut Glfw, window: &mut Window, event: &glfw::WindowEvent);

    /// Short human-readable type name, e.g. `Terrain` or `SkyLight`.
    fn display_name(&self) -> String {
        short_type_name(std::any::type_name::<Self>())
    }

    /// What the component holds in memory, for the RAM map
    /// (`core::memory`): counts and estimated bytes of its big parts.
    /// Nothing by default; every component is counted by type anyway.
    fn memory_report(&self, _report: &mut crate::core::memory::Report) {}

    /// Serializable description used for scene files and play-mode
    /// snapshots. Components that return `None` are either rebuilt by a
    /// prefab (see `ComponentData::Player`) or not persisted at all.
    fn to_data(&self) -> Option<ComponentData> {
        None
    }

    /// Fields exposed to tools such as the editor inspector.
    fn properties(&self) -> Vec<Property> {
        Vec::new()
    }

    /// Apply a value edited in a tool. Unknown names are ignored.
    fn set_property(&mut self, _name: &str, _value: PropertyValue) {}

    /// Update while the editor is in edit mode (simulation stopped). Only
    /// work that keeps the scene presentable belongs here, e.g. streaming
    /// terrain chunks or refitting the shadow box to the editor camera.
    fn update_in_editor(&mut self, _scene: &mut Scene, _entity: &mut Entity, _delta_time: f64) {}

    /// Lights the component gives besides `Light` components (a terrain's
    /// tunnel lamps), for a camera at `eye`: added to `out`. `transform` is
    /// the entity's world transform.
    fn lights(
        &self,
        _transform: &Matrix4<f32>,
        _eye: Point3<f32>,
        _out: &mut Vec<crate::core::renderer::light::light::LightSource>,
    ) {
    }

    /// Bodies whose colliders the user asked to see (the component's "show
    /// collider" toggle). Drawn by tools as wireframes.
    fn visible_collider_bodies(&self, _entity: &Entity) -> Vec<rapier3d::prelude::RigidBodyHandle> {
        Vec::new()
    }

    /// Distance along the ray (`dir` normalized) to the nearest point where
    /// it hits what the component draws, for picking in tools. `transform`
    /// is the one `render` is given.
    fn raycast(
        &self,
        _transform: &Matrix4<f32>,
        _origin: Point3<f32>,
        _dir: Vector3<f32>,
    ) -> Option<f32> {
        None
    }

    /// Release resources owned outside the component (physics bodies,
    /// colliders) when its entity is removed from the scene.
    fn on_remove(&mut self, _physics: &mut PhysicsEngine) {}
}

fn short_type_name(full: &str) -> String {
    let base = full.split('<').next().unwrap_or(full);
    base.rsplit("::").next().unwrap_or(base).to_string()
}

pub mod animation_component;
pub mod camera_component;
pub mod debug_component;
pub mod foot_ik_component;
pub mod held_item_component;
pub mod missing_component;
pub mod model_component;
pub mod shape_component;
pub mod shared_model_component;
mod property;
