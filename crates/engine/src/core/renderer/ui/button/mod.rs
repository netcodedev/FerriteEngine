use crate::core::{renderer::plane::Plane, scene::Scene};

use super::UIElement;

pub mod button;

pub struct Button {
    pub position: (f32, f32),
    pub size: (f32, f32),
    pub on_click: Box<dyn Fn(&mut Scene)>,
    pub children: Vec<Box<dyn UIElement>>,
    pub offset: (f32, f32),
    pub is_hovering: bool,
    plane: Plane,
}

pub struct ButtonBuilder {
    position: (f32, f32),
    size: (f32, f32),
    on_click: Box<dyn Fn(&mut Scene)>,
    children: Vec<Box<dyn UIElement>>,
}
