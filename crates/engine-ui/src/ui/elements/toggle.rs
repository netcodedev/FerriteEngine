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

/// On/off switch bound to a `DataSource<bool>`.
pub struct Toggle {
    handle: UIElementHandle,
    region: Region,
    value: DataSource<bool>,
    is_hovering: bool,
    track: Plane,
    knob: Plane,
}

impl Toggle {
    pub fn new(position: Position, value: DataSource<bool>) -> Self {
        let size = Size::new(36.0, 18.0);
        let track = PlaneBuilder::new()
            .position(position)
            .size(size)
            .color((0.2, 0.2, 0.2, 1.0))
            .border_color((0.5, 0.5, 0.5, 1.0))
            .border_thickness(1.0)
            .border_radius_uniform(9.0)
            .build();
        let knob = PlaneBuilder::new()
            .position(&position + (2.0, 2.0, 1.0))
            .size(Size::new(14.0, 14.0))
            .color((0.85, 0.85, 0.85, 1.0))
            .border_radius_uniform(7.0)
            .build();
        Self {
            handle: UIElementHandle::new(),
            region: Region::new(position, size),
            value,
            is_hovering: false,
            track,
            knob,
        }
    }

    fn sync_visual(&mut self) {
        let base = &self.region.position + &self.region.offset;
        let on = self.value.read();
        let x_off = if on { 20.0 } else { 2.0 };
        self.knob.set_position(&base + (x_off, 2.0, 1.0));
        self.track.set_color(if on {
            (0.25, 0.55, 0.35, 1.0)
        } else {
            (0.2, 0.2, 0.2, 1.0)
        });
    }
}

impl UIElement for Toggle {
    fn update(&mut self, _scene: &mut Scene) {
        self.sync_visual();
    }

    fn render(&self) {
        self.track.render();
        self.knob.render();
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
                    let v = self.value.read();
                    self.value.write(!v);
                    self.sync_visual();
                    return true;
                }
                false
            }
            CursorPos(x, y) => {
                let inside = self.region.contains(*x as f32, *y as f32);
                if inside && !self.is_hovering {
                    window.set_cursor(Some(glfw::Cursor::standard(glfw::StandardCursor::Hand)));
                    self.is_hovering = true;
                } else if !inside && self.is_hovering {
                    window.set_cursor(None);
                    self.is_hovering = false;
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
        self.track
            .set_position(&self.region.position + &self.region.offset);
        self.sync_visual();
    }
    fn get_size(&self) -> &Size {
        &self.region.size
    }
    fn set_z_index(&mut self, z_index: f32) {
        self.region.position.z = z_index;
        self.track.set_z_index(z_index);
        self.knob.set_z_index(z_index + 1.0);
    }
}
