pub mod input;

pub struct Input {
    pub position: (f32, f32),
    pub size: (f32, f32),
    pub offset: (f32, f32),
    pub is_hovering: bool,
    pub is_focused: bool,
    pub content: String,
}

pub struct InputBuilder {
    position: (f32, f32),
    size: (f32, f32),
    content: String,
}