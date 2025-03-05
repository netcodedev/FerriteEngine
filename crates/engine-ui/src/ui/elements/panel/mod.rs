use ferrite::core::{primitives::Region, renderer::plane::Plane, utils::DataSource};

use crate::ui::element_handle::UIElementHandle;

use super::{container::Container, text::Text};

mod panel;

pub struct Panel {
    handle: UIElementHandle,
    region: Region,

    title: Text,
    title_source: Option<DataSource<String>>,
    content: Container,
    controls: Container,

    is_collapsible: bool,
    is_movable: bool,
    has_controls: bool,

    is_hovering: bool,
    is_open: bool,
    is_dragging: bool,
    is_moved: bool,

    drag_position: Option<(f32, f32)>,

    header_plane: Plane,
    plane: Plane,
}
