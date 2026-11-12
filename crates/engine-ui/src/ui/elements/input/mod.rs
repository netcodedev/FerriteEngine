use std::time::Instant;

use ferrite::core::{
    primitives::Region,
    renderer::{plane::Plane, text::Text},
    utils::DataSource,
};

use crate::ui::element_handle::UIElementHandle;

mod edit;
mod input;

pub use edit::TextEdit;

pub struct Input<T: Clone + ToString> {
    handle: UIElementHandle,

    region: Region,
    data_source: Option<DataSource<T>>,
    edit: TextEdit,
    text: Text,

    is_hovering: bool,
    is_focused: bool,
    /// Mouse button held after a click inside: dragging extends the
    /// selection.
    dragging: bool,
    last_click: Option<(Instant, u8)>,

    scroll_offset: f32,
    blink_start: Instant,

    plane: Plane,
    stencil_plane: Plane,
    cursor_plane: Plane,
    selection_plane: Plane,
}
