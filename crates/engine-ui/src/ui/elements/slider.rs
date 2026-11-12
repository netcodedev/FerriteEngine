use crate::ui::theme;
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

/// Diameter of the round handle.
const KNOB: f32 = 14.0;

/// Horizontal slider bound to a `DataSource<f32>` in `[min, max]`.
pub struct Slider {
    handle: UIElementHandle,
    region: Region,
    value: DataSource<f32>,
    min: f32,
    max: f32,
    /// Dragging snaps to multiples of this (typed values are not snapped).
    step: Option<f32>,
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
            .color(theme::BG_CONTROL)
            .border_color(theme::BG_CONTROL)
            .border_thickness(1.0)
            .border_radius_uniform(theme::RADIUS_SMALL)
            .build();
        let fill = PlaneBuilder::new()
            .position(&position + (0.0, track_y_off, 1.0))
            .size(Size::new(0.0, track_h))
            .color(theme::ACCENT)
            .border_radius_uniform(theme::RADIUS_SMALL)
            .build();
        let knob = PlaneBuilder::new()
            .position(&position + (0.0, 0.0, 2.0))
            .size(Size::new(KNOB, KNOB))
            .color(theme::TEXT)
            .border_radius_uniform(KNOB * 0.5)
            .build();
        Self {
            handle: UIElementHandle::new(),
            region: Region::new(position, size),
            value,
            min,
            max,
            step: Some(default_step(min, max)),
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

    /// Snap dragged values to `step`; `None` keeps them continuous.
    pub fn set_step(&mut self, step: Option<f32>) {
        self.step = step.filter(|s| *s > 0.0);
    }

    fn set_from_cursor(&mut self, cursor_x: f32) {
        let origin_x = self.region.position.x + self.region.offset.x;
        let f = ((cursor_x - origin_x) / self.region.size.width).clamp(0.0, 1.0);
        let raw = self.min + f * (self.max - self.min);
        self.value.write(snap(raw, self.min, self.max, self.step));
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
        let knob_y = (self.region.size.height - KNOB) * 0.5;
        self.knob
            .set_position(&base + (fill_w - KNOB * 0.5, knob_y, 2.0));
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

/// A "nice" step of about a hundredth of the range: 0..1 -> 0.01,
/// 0..2 -> 0.01, 10..120 -> 1.
pub fn default_step(min: f32, max: f32) -> f32 {
    let range = (max - min).abs().max(f32::EPSILON);
    10f32.powf((range / 100.0).log10().floor())
}

/// Round `value` to the nearest multiple of `step` from `min`, cleaned of
/// float noise (0.35000002 -> 0.35) and kept inside `[min, max]`.
pub fn snap(value: f32, min: f32, max: f32, step: Option<f32>) -> f32 {
    let Some(step) = step else {
        return value.clamp(min, max);
    };
    let steps = ((value - min) / step).round();
    let decimals = (-step.log10().floor()).max(0.0) as i32;
    let factor = 10f32.powi(decimals);
    let snapped = ((min + steps * step) * factor).round() / factor;
    snapped.clamp(min.min(max), max.max(min))
}

#[cfg(test)]
mod tests {
    use super::{default_step, snap};

    #[test]
    fn default_steps_scale_with_the_range() {
        assert_eq!(default_step(0.0, 1.0), 0.01);
        assert_eq!(default_step(0.0, 2.0), 0.01);
        assert_eq!(default_step(10.0, 120.0), 1.0);
        assert_eq!(default_step(-89.0, 89.0), 1.0);
    }

    #[test]
    fn snapping_rounds_to_clean_values() {
        assert_eq!(snap(0.123456, 0.0, 1.0, Some(0.01)).to_string(), "0.12");
        assert_eq!(snap(0.355, 0.0, 1.0, Some(0.01)).to_string(), "0.36");
        assert_eq!(snap(64.4, 10.0, 120.0, Some(1.0)), 64.0);
        assert_eq!(snap(1.5, 0.0, 1.0, Some(0.01)), 1.0);
        assert_eq!(snap(0.123456, 0.0, 1.0, None), 0.123456);
    }
}
