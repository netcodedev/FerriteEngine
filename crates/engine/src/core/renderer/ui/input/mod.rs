use crate::core::{
    renderer::{plane::Plane, text::Text},
    utils::DataSource,
};

use super::{offset::Offset, position::Position, size::Size};

pub mod input;

pub struct Input<T: Clone + ToString> {
    position: Position,
    size: Size,
    offset: Offset,
    pub is_hovering: bool,
    pub is_focused: bool,
    pub content: String,
    text: Text,
    plane: Plane,
    stencil_plane: Plane,
    data_source: Option<DataSource<T>>,
}

pub struct InputBuilder<T: Clone + ToString> {
    position: Position,
    size: Size,
    content: T,
    data_source: Option<DataSource<T>>,
}
