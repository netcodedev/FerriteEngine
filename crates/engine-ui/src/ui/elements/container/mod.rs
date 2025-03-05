use std::collections::BTreeMap;

use ferrite::core::{primitives::Region, renderer::plane::Plane};

use crate::ui::{element_handle::UIElementHandle, UIElement};

mod container;

pub enum Direction {
    Horizontal,
    Vertical,
}

pub struct Container {
    handle: UIElementHandle,

    region: Region,
    children: BTreeMap<UIElementHandle, Box<dyn UIElement>>,

    gap: f32,
    with_end_gap: bool,
    direction: Direction,

    plane: Plane,
}
