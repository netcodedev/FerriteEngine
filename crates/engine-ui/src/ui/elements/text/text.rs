use ferrite::core::renderer::text::Fonts;

use ferrite::core::primitives::{Offset, Position, Region, Size};
use ferrite::core::scene::Scene;
use glfw::{Glfw, Window, WindowEvent};

use crate::ui::element_handle::UIElementHandle;
use crate::ui::UIElement;

use super::Text;

impl Text {
    pub fn new(text: String, size: f32) -> Self {
        Self {
            handle: UIElementHandle::new(),
            region: Region {
                size: Size {
                    width: size * text.len() as f32, // initial estimate (will be too high)
                    height: size,
                },
                ..Default::default()
            },
            content: text.clone(),
            text: ferrite::core::renderer::text::Text::new(
                Fonts::RobotoMono,
                0,
                0,
                0,
                size,
                text.to_string(),
            ),
        }
    }

    pub fn set_handle(&mut self, handle: UIElementHandle) {
        self.handle = handle;
    }

    pub fn set_position(&mut self, position: Position) {
        self.region.position = position;
    }

    pub fn set_text(&mut self, text: String) {
        self.content = text.clone();
        self.text.set_content(&text);
        self.region.size.width = self.text.get_size().width;
    }

    pub fn get_text(&self) -> String {
        self.content.clone()
    }
}

impl UIElement for Text {
    fn update(&mut self, _scene: &mut Scene) {
        self.text.set_content(&self.content);
        self.text.prepare_render_at(self.region.get_absolute_position());
        self.region.size = self.text.get_size();
    }

    fn render(&self) {
        self.text.render();
    }

    fn handle_events(
        &mut self,
        _scene: &mut Scene,
        _window: &mut Window,
        _glfw: &mut Glfw,
        _event: &WindowEvent,
    ) -> bool {
        false
    }

    fn add_child(&mut self, _child: Box<dyn UIElement>) {
        panic!("Text cannot have children");
    }

    fn add_child_to(&mut self, _parent: UIElementHandle, _child: Box<dyn UIElement>) {
        panic!("Text cannot have children");
    }

    fn contains_child(&self, _handle: &UIElementHandle) -> bool {
        false
    }

    fn get_child(&self, _handle: &UIElementHandle) -> Option<&Box<dyn UIElement>> {
        None
    }

    fn get_child_mut(&mut self, _handle: &UIElementHandle) -> Option<&mut Box<dyn UIElement>> {
        None
    }

    fn get_handle(&self) -> &UIElementHandle {
        &self.handle
    }

    fn get_offset(&self) -> &Offset {
        &self.region.offset
    }

    fn set_offset(&mut self, offset: Offset) {
        self.region.offset = offset;
    }

    fn get_size(&self) -> &Size {
        &self.region.size
    }

    fn set_z_index(&mut self, z_index: f32) {
        self.region.position.z = z_index;
        self.text.set_z_index(z_index);
    }
}
