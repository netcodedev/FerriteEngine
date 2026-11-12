use ferrite::core::{
    primitives::{Offset, Position, Region, Size},
    renderer::plane::{Plane, PlaneBuilder},
    scene::Scene,
};
use glfw::{Glfw, Window, WindowEvent};

use crate::ui::{element_handle::UIElementHandle, UIElement};

#[derive(Clone, Copy)]
pub enum Orientation {
    Horizontal,
    Vertical,
}

/// Thin separator line.
pub struct Divider {
    handle: UIElementHandle,
    region: Region,
    plane: Plane,
}

impl Divider {
    pub fn new(position: Position, length: f32, orientation: Orientation) -> Self {
        let thickness = 1.0;
        let size = match orientation {
            Orientation::Horizontal => Size::new(length, thickness),
            Orientation::Vertical => Size::new(thickness, length),
        };
        let plane = PlaneBuilder::new()
            .position(position)
            .size(size)
            .color((0.4, 0.4, 0.4, 1.0))
            .build();
        Self {
            handle: UIElementHandle::new(),
            region: Region::new(position, size),
            plane,
        }
    }
}

impl UIElement for Divider {
    fn update(&mut self, _scene: &mut Scene) {}

    fn render(&self) {
        self.plane.render();
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
        self.plane
            .set_position(&self.region.position + &self.region.offset);
    }
    fn get_size(&self) -> &Size {
        &self.region.size
    }
    fn set_z_index(&mut self, z_index: f32) {
        self.region.position.z = z_index;
        self.plane.set_z_index(z_index);
    }
}
