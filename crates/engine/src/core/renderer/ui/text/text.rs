use crate::core::{
    renderer::{
        text::Fonts,
        ui::{UIElement, UIElementHandle},
    },
    scene::Scene,
};

use super::Text;

impl Text {
    pub fn new(text: String, size: f32) -> Self {
        Self {
            size,
            content: text.clone(),
            text: crate::core::renderer::text::Text::new(
                Fonts::RobotoMono,
                0,
                0,
                size,
                text.to_string(),
            ),
            offset: (0.0, 0.0),
            width: size * text.len() as f32, // initial estimate (will be too high)
        }
    }
}

impl UIElement for Text {
    fn render(&mut self, _: &mut Scene) {
        self.text.set_content(self.content.clone());
        let (width, height) = self
            .text
            .render_at(self.offset.0 as i32 + 5, self.offset.1 as i32 + 2);
        if width as f32 != self.width {
            self.width = width as f32;
        }
        if height as f32 != self.size {
            self.size = height as f32;
        }
    }

    fn handle_events(
        &mut self,
        _: &mut Scene,
        _window: &mut glfw::Window,
        _: &mut glfw::Glfw,
        _event: &glfw::WindowEvent,
    ) -> bool {
        false
    }

    fn add_children(&mut self, _children: Vec<(Option<UIElementHandle>, Box<dyn UIElement>)>) {
        panic!("Text cannot have children");
    }

    fn set_offset(&mut self, offset: (f32, f32)) {
        println!("Setting offset for {}: {:?}", self.content, offset);
        self.offset = offset;
    }

    fn get_size(&self) -> (f32, f32) {
        (self.width, self.size)
    }

    fn contains_child(&self, _: &UIElementHandle) -> bool {
        false
    }

    fn get_offset(&self) -> (f32, f32) {
        self.offset
    }
}
