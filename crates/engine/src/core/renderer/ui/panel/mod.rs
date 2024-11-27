use crate::core::renderer::{plane::Plane, text::Text};

use super::{container::Container, UIElement};

pub mod panel;

pub struct Panel {
    position: (f32, f32, f32),
    offset: (f32, f32),
    size: (f32, f32),
    title: String,
    content: Container,
    text: Text,
    drag_start: Option<(f64, f64)>,
    dragging: bool,
    is_hovering: bool,
    plane: Plane,
    header_plane: Plane,
}

pub struct PanelBuilder {
    pub position: (f32, f32, f32),
    pub size: (f32, f32),
    pub title: String,
    pub children: Vec<Box<dyn UIElement>>,
}
