use std::collections::BTreeMap;

use ferrite::core::{
    primitives::{Offset, Region},
    renderer::{plane::Plane, text::Text},
    scene::Scene,
};

use crate::ui::{element_handle::UIElementHandle, UIElement};

mod button;

pub struct Button {
    handle: UIElementHandle,

    region: Region,
    padding: Offset,

    on_click: Box<dyn Fn(&mut Scene)>,
    children: BTreeMap<UIElementHandle, Box<dyn UIElement>>,

    is_hovering: bool,
    is_active: bool,
    style: ButtonStyle,

    label: Option<(String, Text)>,
    icon: Option<(String, Text)>,
    plane: Plane,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonStyle {
    /// Filled control surface with a subtle border (default).
    Secondary,
    /// No background until hovered; for toolbars and icon actions.
    Ghost,
    /// Accent-filled call to action.
    Primary,
    /// Ghost that turns red on hover (close, delete).
    Danger,
}
