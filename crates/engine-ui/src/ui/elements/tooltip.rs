use ferrite::core::{
    primitives::{Offset, Position, Region, Size},
    renderer::{
        plane::{Plane, PlaneBuilder},
        text::{Fonts, Text as FerriteText},
    },
    scene::Scene,
};
use glfw::{
    Glfw, Window,
    WindowEvent::{self, CursorPos},
};

use crate::ui::{element_handle::UIElementHandle, UIElement};

/// Floating label that appears when the cursor is inside `trigger`.
pub struct Tooltip {
    handle: UIElementHandle,
    region: Region,
    trigger: Region,
    content: String,
    visible: bool,
    background: Plane,
    text: FerriteText,
    padding: (f32, f32),
}

impl Tooltip {
    pub fn new(text: impl Into<String>, trigger: Region) -> Self {
        let content: String = text.into();
        let size = 14.0;
        let padding = (6.0, 3.0);
        let background = PlaneBuilder::new()
            .position(Position::default())
            .size(Size::default())
            .color((0.1, 0.1, 0.1, 0.9))
            .border_color((0.5, 0.5, 0.5, 1.0))
            .border_thickness(1.0)
            .border_radius_uniform(3.0)
            .build();
        Self {
            handle: UIElementHandle::new(),
            region: Region::default(),
            trigger,
            text: FerriteText::new(Fonts::RobotoMono, 0, 0, 0, size, content.clone()),
            content,
            visible: false,
            background,
            padding,
        }
    }
}

impl UIElement for Tooltip {
    fn update(&mut self, _scene: &mut Scene) {
        self.text.set_content(&self.content);
        let pos = &self.region.position + &self.region.offset;
        self.text.prepare_render_at(&pos + (self.padding.0, self.padding.1, 1.0));
        let text_size = self.text.get_size();
        self.region.size = Size::new(
            text_size.width + self.padding.0 * 2.0,
            text_size.height + self.padding.1 * 2.0,
        );
        self.background.set_position(pos);
        self.background.set_size(self.region.size);
    }

    fn render(&self) {
        if self.visible {
            self.background.render();
            self.text.render();
        }
    }

    fn handle_events(
        &mut self,
        _scene: &mut Scene,
        _window: &mut Window,
        _glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        if let CursorPos(x, y) = event {
            let x = *x as f32;
            let y = *y as f32;
            self.visible = self.trigger.contains(x, y);
            if self.visible {
                self.region.position = Position::new(x + 12.0, y + 12.0, self.region.position.z);
            }
        }
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
        self.background.set_z_index(z_index);
        self.text.set_z_index(z_index + 1.0);
    }
}
