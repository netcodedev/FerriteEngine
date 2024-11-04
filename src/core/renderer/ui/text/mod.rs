pub mod text;

pub struct Text {
    pub content: String,
    text: crate::core::renderer::text::Text,
    pub size: f32,
    pub offset: (f32, f32),
    pub width: f32,
}
