use ferrite::core::{
    primitives::{Offset, Region, Size},
    renderer::text::{Fonts, Text as FerriteText},
    scene::Scene,
    utils::DataSource,
};
use glfw::{Glfw, Window, WindowEvent};

use crate::ui::{element_handle::UIElementHandle, UIElement};

/// Static or data-bound text label. Non-interactive.
pub struct Label {
    handle: UIElementHandle,
    region: Region,
    content: String,
    source: Option<DataSource<String>>,
    text: FerriteText,
}

impl Label {
    pub fn new(text: impl Into<String>, size: f32) -> Self {
        let content: String = text.into();
        Self {
            handle: UIElementHandle::new(),
            region: Region {
                size: Size::new(size * content.len() as f32, size),
                ..Default::default()
            },
            text: {
                let mut t = FerriteText::new(Fonts::Inter, 0, 0, 0, size, content.clone());
                t.set_color(crate::ui::theme::TEXT);
                t
            },
            content,
            source: None,
        }
    }

    pub fn bound(source: DataSource<String>, size: f32) -> Self {
        let mut l = Self::new(source.read(), size);
        l.source = Some(source);
        l
    }

    pub fn set_color(&mut self, color: (f32, f32, f32, f32)) {
        self.text.set_color(color);
    }

    pub fn set_font(&mut self, font: Fonts, size: f32) {
        self.text.set_font(font, size);
    }

    /// Dimmed secondary text.
    pub fn muted(text: impl Into<String>, size: f32) -> Self {
        let mut label = Self::new(text, size);
        label.set_color(crate::ui::theme::TEXT_MUTED);
        label
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.content = text.into();
        self.text.set_content(&self.content);
    }
}

impl UIElement for Label {
    fn update(&mut self, _scene: &mut Scene) {
        if let Some(src) = &self.source {
            self.content = src.read();
        }
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

    leaf_children_methods!();

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
