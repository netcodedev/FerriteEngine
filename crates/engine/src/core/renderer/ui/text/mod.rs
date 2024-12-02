use super::Offset;

pub mod text;

pub struct Text {
    pub content: String,
    text: crate::core::renderer::text::Text,
    pub size: f32,
    pub offset: Offset,
    pub width: f32,
}
