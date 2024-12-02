use std::collections::BTreeMap;

use crate::core::renderer::plane::Plane;

use super::{
    primitives::{Offset, Position, Size},
    UIElement, UIElementHandle,
};

pub mod container;

pub struct Container {
    pub position: Position,
    size: Size,
    pub children: BTreeMap<UIElementHandle, Box<dyn UIElement>>,
    pub offset: Offset,
    gap: f32,
    plane: Plane,
    y_offset: f32,
}

pub struct ContainerBuilder {
    position: Position,
    size: Size,
    children: Vec<(Option<UIElementHandle>, Box<dyn UIElement>)>,
}
