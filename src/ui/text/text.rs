use crate::{text::TextRenderer, ui::UIElement};

use super::Text;

impl Text {
    pub fn new(text: &str, size: f32) -> Self {
        Self {
            size,
            text: text.to_string(),
            offset: (0.0, 0.0),
            width: size * text.len() as f32, // initial estimate (will be too high)
        }
    }
}

impl UIElement for Text {
    fn render(&mut self) {
        let (width, _) = TextRenderer::render(
            self.offset.0 as i32 + 5,
            self.offset.1 as i32 + 2,
            self.size,
            self.text.as_str(),
        );
        if width as f32 != self.width {
            self.width = width as f32;
        }
    }

    fn handle_events(
        &mut self,
        _window: &mut glfw::Window,
        _: &mut glfw::Glfw,
        _event: &glfw::WindowEvent,
    ) -> bool {
        false
    }

    fn add_children(&mut self, _children: Vec<Box<dyn UIElement>>) {}

    fn set_offset(&mut self, offset: (f32, f32)) {
        self.offset = offset;
    }

    fn get_size(&self) -> (f32, f32) {
        (self.width, self.size)
    }
}
