use component::Component;

mod entity;
pub mod component;

pub struct Entity {
    children: Vec<Entity>,
    components: Vec<Box<dyn Component>>,
}