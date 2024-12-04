use std::collections::BTreeMap;

use crate::core::{renderer::plane::Plane, scene::Scene};

use super::{offset::Offset, position::Position, size::Size, UIElement, UIElementHandle};

pub mod button;

pub struct Button {
    pub position: Position,
    pub size: Size,
    pub on_click: Box<dyn Fn(&mut Scene)>,
    pub children: BTreeMap<UIElementHandle, Box<dyn UIElement>>,
    pub offset: Offset,
    pub is_hovering: bool,
    plane: Plane,
}

pub struct ButtonBuilder {
    position: Position,
    size: Size,
    on_click: Box<dyn Fn(&mut Scene)>,
    children: Vec<(Option<UIElementHandle>, Box<dyn UIElement>)>,
}
