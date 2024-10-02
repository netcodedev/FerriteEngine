use super::UIElement;

pub mod container;

pub struct Container {
    pub position: (f32, f32),
    pub size: (f32, f32),
    pub children: Vec<Box<dyn UIElement>>,
    pub offset: (f32, f32),
    gap: f32,
}

pub struct ContainerBuilder {
    position: (f32, f32),
    size: (f32, f32),
    children: Vec<Box<dyn UIElement>>,
}