use std::collections::BTreeMap;

use ferrite::core::{primitives::{Offset, Region}, renderer::plane::Plane, scene::Scene};

use crate::ui::{element_handle::UIElementHandle, UIElement};

mod button;

pub struct Button {
    handle: UIElementHandle,

    region: Region,
    padding: Offset,

    on_click: Box<dyn Fn(&mut Scene)>,
    children: BTreeMap<UIElementHandle, Box<dyn UIElement>>,

    is_hovering: bool,

    plane: Plane,
}
