use cgmath::Point3;
use component::Component;

pub mod component;
mod entity;

pub struct Entity {
    children: Vec<Entity>,
    components: Vec<Box<dyn Component>>,
    position: Point3<f32>,
}
