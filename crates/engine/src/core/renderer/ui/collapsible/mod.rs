use crate::core::renderer::{plane::Plane, text::Text};

use super::container::Container;

mod collapsible;

pub struct Collapsible {
    is_open: bool,
    title: String,
    text: Text,
    content: Container,
    offset: (f32, f32),
    position: (f32, f32, f32),
    plane: Plane,
    header_plane: Plane,
    size: (f32, f32),
}
