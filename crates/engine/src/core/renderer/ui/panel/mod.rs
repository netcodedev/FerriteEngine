use crate::core::renderer::{plane::Plane, text::Text};

use super::{
    container::Container,
    primitives::{Offset, Position, Size},
    UIElement, UIElementHandle,
};

pub mod panel;

pub struct Panel {
    position: Position,
    offset: Offset,
    size: Size,

    title: String,
    content: Container,

    drag_start: Option<Position>,
    dragging: bool,
    is_hovering: bool,
    collapsible: bool,
    movable: bool,
    is_open: bool,
    moved: bool,

    text: Text,
    plane: Plane,
    header_plane: Plane,
}

pub struct PanelBuilder {
    pub position: Position,
    pub size: Size,
    pub title: String,
    pub children: Vec<(Option<UIElementHandle>, Box<dyn UIElement>)>,
    pub collapsible: bool,
    pub movable: bool,
    pub open: bool,
}
