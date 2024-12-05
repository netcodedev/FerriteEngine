use std::collections::BTreeMap;

use crate::core::renderer::plane::Plane;

use super::{primitives::Position, Offset, Size, UIElement, UIElementHandle};

pub mod container;

pub enum Direction {
    Horizontal,
    Vertical,
}

pub struct Container {
    pub position: Position,
    size: Size,
    pub children: BTreeMap<UIElementHandle, Box<dyn UIElement>>,
    pub offset: Offset,
    gap: f32,
    plane: Plane,
    direction: Direction,

    with_end_gap: bool,
}

pub struct ContainerBuilder {
    position: Position,
    size: Size,
    children: Vec<(Option<UIElementHandle>, Box<dyn UIElement>)>,
    with_end_gap: bool,
    direction: Direction,
}
