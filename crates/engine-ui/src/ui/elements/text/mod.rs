use ferrite::core::primitives::Region;

use crate::ui::element_handle::UIElementHandle;

pub mod text;

pub struct Text {
    handle: UIElementHandle,
    content: String,
    text: ferrite::core::renderer::text::Text,
    region: Region,
}
