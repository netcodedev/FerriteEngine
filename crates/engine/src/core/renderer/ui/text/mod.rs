use super::{size::Size, Offset};

pub mod text;

pub struct Text {
    pub content: String,
    text: crate::core::renderer::text::Text,
    pub size: Size,
    pub offset: Offset,
}
