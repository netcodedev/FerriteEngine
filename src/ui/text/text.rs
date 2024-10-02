use crate::{plane::PlaneRenderer, text::TextRenderer, ui::UIElement};

use super::Text;

impl Text {
    pub fn new(text: &str, size: f32) -> Self {
        Self {
            size,
            text: text.to_string(),
            offset: (0.0, 0.0),
        }
    }
}

impl UIElement for Text {
    fn render(&self, text_renderer: &mut TextRenderer, _: &PlaneRenderer) {
        text_renderer.render(self.offset.0 as u32 + 5, self.offset.1 as u32 + 2, self.size, self.text.as_str());
    }

    fn handle_events(&mut self, _window: &mut glfw::Window, _event: &glfw::WindowEvent) -> bool {
        false
    }

    fn add_children(&mut self, _children: Vec<Box<dyn UIElement>>) {}

    fn set_offset(&mut self, offset: (f32, f32)) {
        self.offset = offset;
    }
}