use ferrite::core::{
    primitives::{Offset, Position, Region, Size},
    renderer::plane::{Plane, PlaneBuilder},
    scene::Scene,
    utils::DataSource,
};
use glfw::{
    Action::{Press, Release},
    Glfw, MouseButtonLeft, Window,
    WindowEvent::{self, CursorPos, MouseButton},
};

use crate::ui::{element_handle::UIElementHandle, UIElement};

/// Horizontal slider bound to a `DataSource<f32>` in `[min, max]`.
pub struct Slider {
    handle: UIElementHandle,
    region: Region,
    value: DataSource<f32>,
    min: f32,
    max: f32,
    is_hovering: bool,
    is_dragging: bool,
    track: Plane,
    fill: Plane,
    knob: Plane,
}

impl Slider {
    pub fn new(position: Position, size: Size, value: DataSource<f32>, min: f32, max: f32) -> Self {
        let track_h = 4.0;
        let track_y_off = (size.height - track_h) * 0.5;
        let track = PlaneBuilder::new()
            .position(&position + (0.0, track_y_off))
            .size(Size::new(size.width, track_h))
            .color((0.2, 0.2, 0.2, 1.0))
            .border_color((0.5, 0.5, 0.5, 1.0))
            .border_thickness(1.0)
            .border_radius_uniform(2.0)
            .build();
        let fill = PlaneBuilder::new()
            .position(&position + (0.0, track_y_off, 1.0))
            .size(Size::new(0.0, track_h))
            .color((0.3, 0.55, 0.8, 1.0))
            .border_radius_uniform(2.0)
            .build();
        let knob = PlaneBuilder::new()
            .position(&position + (0.0, 0.0, 2.0))
            .size(Size::new(10.0, size.height))
            .color((0.85, 0.85, 0.85, 1.0))
            .border_radius_uniform(3.0)
            .build();
        Self {
            handle: UIElementHandle::new(),
            region: Region::new(position, size),
            value,
            min,
            max,
            is_hovering: false,
            is_dragging: false,
            track,
            fill,
            knob,
        }
    }

    fn fraction(&self) -> f32 {
        let v = self.value.read();
        ((v - self.min) / (self.max - self.min)).clamp(0.0, 1.0)
    }

    fn set_from_cursor(&mut self, cursor_x: f32) {
        let origin_x = self.region.position.x + self.region.offset.x;
        let f = ((cursor_x - origin_x) / self.region.size.width).clamp(0.0, 1.0);
        self.value.write(self.min + f * (self.max - self.min));
    }

    fn sync_visual(&mut self) {
        let base = &self.region.position + &self.region.offset;
        let track_h = 4.0;
        let track_y_off = (self.region.size.height - track_h) * 0.5;
        let f = self.fraction();
        let fill_w = self.region.size.width * f;
        self.track.set_position(&base + (0.0, track_y_off));
        self.fill.set_position(&base + (0.0, track_y_off, 1.0));
        self.fill.set_size(Size::new(fill_w, track_h));
        self.knob.set_position(&base + (fill_w - 5.0, 0.0, 2.0));
    }
}

impl UIElement for Slider {
    fn update(&mut self, _scene: &mut Scene) {
        self.sync_visual();
    }

    fn render(&self) {
        self.track.render();
        self.fill.render();
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
                    self.is_dragging = true;
                    self.set_from_cursor(x as f32);
                    self.sync_visual();
                    return true;
                }
                false
            }
            MouseButton(MouseButtonLeft, Release, _) => {
                let was = self.is_dragging;
                self.is_dragging = false;
                was
            }
            CursorPos(x, y) => {
                if self.is_dragging {
                    self.set_from_cursor(*x as f32);
                    self.sync_visual();
                    return true;
                }
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
        self.sync_visual();
    }
    fn get_size(&self) -> &Size {
        &self.region.size
    }
    fn set_z_index(&mut self, z_index: f32) {
        self.region.position.z = z_index;
        self.track.set_z_index(z_index);
        self.fill.set_z_index(z_index + 1.0);
        self.knob.set_z_index(z_index + 2.0);
    }
}
