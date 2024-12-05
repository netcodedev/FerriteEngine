use crate::core::renderer::plane::Plane;

use super::panel::Panel;

mod popup;

pub struct Popup {
    background: Plane,
    panel: Panel,
}
