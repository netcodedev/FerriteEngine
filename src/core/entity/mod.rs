use component::Component;

pub mod component;
mod entity;

pub struct Entity {
    children: Vec<Entity>,
    components: Vec<Box<dyn Component>>,
}
