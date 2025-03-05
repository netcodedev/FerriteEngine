use ferrite::core::{
    primitives::Region,
    renderer::{plane::Plane, text::Text},
    utils::DataSource,
};

use crate::ui::element_handle::UIElementHandle;

mod input;

pub struct Input<T: Clone + ToString> {
    handle: UIElementHandle,

    region: Region,
    data_source: Option<DataSource<T>>,
    content: String,
    text: Text,

    is_hovering: bool,
    is_focused: bool,

    plane: Plane,
    stencil_plane: Plane,
}
