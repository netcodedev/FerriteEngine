use ferrite::core::{
    primitives::{Offset, Position, Region, Size},
    renderer::plane::{Plane, PlaneBuilder},
    scene::Scene,
    utils::DataSource,
};
use glfw::{
    Action::Press, Glfw, MouseButtonLeft, Window,
    WindowEvent::{self, CursorPos, MouseButton},
};

use crate::ui::{element_handle::UIElementHandle, UIElement};

/// One radio button. Selected when `value.read() == index`.
pub struct Radio {
    handle: UIElementHandle,
    region: Region,
    value: DataSource<u32>,
    index: u32,
    is_hovering: bool,
    outer: Plane,
    inner: Plane,
}

impl Radio {
    pub fn new(position: Position, value: DataSource<u32>, index: u32) -> Self {
        let size = Size::new(18.0, 18.0);
        let outer = PlaneBuilder::new()
            .position(position)
            .size(size)
            .color((0.15, 0.15, 0.15, 1.0))
            .border_color((0.6, 0.6, 0.6, 1.0))
            .border_thickness(1.0)
            .border_radius_uniform(9.0)
            .build();
        let inner = PlaneBuilder::new()
            .position(&position + (4.0, 4.0, 1.0))
            .size(Size::new(10.0, 10.0))
            .color((0.3, 0.7, 0.4, 1.0))
            .border_radius_uniform(5.0)
            .build();
        Self {
            handle: UIElementHandle::new(),
            region: Region::new(position, size),
            value,
            index,
            is_hovering: false,
            outer,
            inner,
        }
    }
}

impl UIElement for Radio {
    fn update(&mut self, _scene: &mut Scene) {}

    fn render(&self) {
        self.outer.render();
        if self.value.read() == self.index {
            self.inner.render();
        }
    }

    fn handle_events(
        &mut self,
        _scene: &mut Scene,
        window: &mut Window,
        _glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        match event {
            MouseButton(MouseButtonLeft, Press, _) => {
                let (x, y) = window.get_cursor_pos();
                if self.region.contains(x as f32, y as f32) {
                    self.value.write(self.index);
                    return true;
                }
                false
            }
            CursorPos(x, y) => {
                let inside = self.region.contains(*x as f32, *y as f32);
                if inside && !self.is_hovering {
                    window.set_cursor(Some(glfw::Cursor::standard(glfw::StandardCursor::Hand)));
                    self.is_hovering = true;
                    self.outer.set_color((0.22, 0.22, 0.22, 1.0));
                } else if !inside && self.is_hovering {
                    window.set_cursor(None);
                    self.is_hovering = false;
                    self.outer.set_color((0.15, 0.15, 0.15, 1.0));
                }
                false
            }
            _ => false,
        }
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
        let pos = &self.region.position + &self.region.offset;
        self.outer.set_position(pos);
        self.inner.set_position(&pos + (4.0, 4.0, 1.0));
    }
    fn get_size(&self) -> &Size {
        &self.region.size
    }
    fn set_z_index(&mut self, z_index: f32) {
        self.region.position.z = z_index;
        self.outer.set_z_index(z_index);
        self.inner.set_z_index(z_index + 1.0);
    }
}
