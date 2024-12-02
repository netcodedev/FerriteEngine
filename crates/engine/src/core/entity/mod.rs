use cgmath::{Point3, Quaternion};
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
}
