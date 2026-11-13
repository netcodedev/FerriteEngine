use cgmath::{Point3, Quaternion, Vector3};
use component::Component;

use super::utils::DataSource;

pub mod component;
mod entity;
mod entity_handle;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntityHandle(u64);

pub struct Entity {
    pub id: EntityHandle,
    name: DataSource<String>,
    children: Vec<Entity>,
    components: Vec<Box<dyn Component>>,
    position: Point3<f32>,
    rotation: Quaternion<f32>,
    /// Per local axis. Affects what the entity and its children draw, not
    /// physics shapes.
    scale: Vector3<f32>,
    /// Runtime-generated entities (terrain chunks, …) that are rebuilt by
    /// their owner. They are neither saved nor listed in the editor.
    transient: bool,
}
