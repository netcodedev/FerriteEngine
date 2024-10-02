use super::{container::Container, UIElement};

pub mod panel;

pub struct Panel {
    pub position: (f32, f32, f32),
    pub offset: (f32, f32),
    pub size: (f32, f32),
    pub title: String,
    content: Container,
    drag_start: Option<(f64, f64)>,
    dragging: bool
}

pub struct PanelBuilder {
    pub position: (f32, f32, f32),
    pub size: (f32, f32),
    pub title: String,
    pub children: Vec<Box<dyn UIElement>>,
}