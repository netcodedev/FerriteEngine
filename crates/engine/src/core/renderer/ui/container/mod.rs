use std::collections::BTreeMap;

use crate::core::renderer::plane::Plane;

use super::{UIElement, UIElementHandle};

pub mod container;

pub struct Container {
    pub position: (f32, f32),
    size: (f32, f32),
    pub children: BTreeMap<UIElementHandle, Box<dyn UIElement>>,
    pub offset: (f32, f32),
    gap: f32,
    plane: Plane,
    y_offset: f32,
}

pub struct ContainerBuilder {
    position: (f32, f32),
    size: (f32, f32),
    children: Vec<(Option<UIElementHandle>, Box<dyn UIElement>)>,
}
