use crate::core::{
    renderer::{
        text::Fonts,
        ui::{offset::Offset, size::Size, UIElement, UIElementHandle},
    },
    scene::Scene,
};

use super::Text;

impl Text {
    pub fn new(text: String, size: f32) -> Self {
        Self {
            size: Size {
                width: size * text.len() as f32, // initial estimate (will be too high)
                height: size,
            },
            content: text.clone(),
            text: crate::core::renderer::text::Text::new(
                Fonts::RobotoMono,
                0,
                0,
                size,
                text.to_string(),
            ),
            offset: Offset::default(),
        }
    }
}

impl UIElement for Text {
    fn render(&mut self, _: &mut Scene) {
        self.text.set_content(&self.content);
        let (width, height) = self.text.render_at((&self.offset + (5.0, 2.0)).into());
        if width as f32 != self.size.width {
            self.size.width = width as f32;
        }
        if height as f32 != self.size.height {
            self.size.height = height as f32;
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

    fn set_offset(&mut self, offset: Offset) {
        self.offset = offset;
    }

    fn get_size(&self) -> &Size {
        &self.size
    }

    fn contains_child(&self, _: &UIElementHandle) -> bool {
        false
    }

    fn get_offset(&self) -> &Offset {
        &self.offset
    }

    fn add_child_to(
        &mut self,
        _: UIElementHandle,
        _: Option<UIElementHandle>,
        _: Box<dyn UIElement>,
    ) {
        panic!("Text cannot have children");
    }
}
