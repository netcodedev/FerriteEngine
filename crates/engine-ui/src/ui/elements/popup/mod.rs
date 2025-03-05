use ferrite::core::renderer::plane::Plane;

use crate::ui::element_handle::UIElementHandle;

use super::panel::Panel;

mod popup;

pub struct Popup {
    handle: UIElementHandle,
    background: Plane,
    panel: Panel,
}
