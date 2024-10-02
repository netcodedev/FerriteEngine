use crate::{plane::PlaneRenderer, text::TextRenderer, ui::UIElement};

use super::Text;

impl Text {
    pub fn new(text: &str, size: f32) -> Self {
        Self {
            size,
            text: text.to_string(),
        }
    }
}

impl UIElement for Text {
    fn render(&self, text_renderer: &mut TextRenderer, _: &PlaneRenderer) {
        text_renderer.render(33, 40, self.size, self.text.as_str());
    }

    fn handle_events(&self, _window: &mut glfw::Window, _event: &glfw::WindowEvent) -> bool {
        false
    }

    fn add_children(&mut self, _children: Vec<Box<dyn UIElement>>) {}
}