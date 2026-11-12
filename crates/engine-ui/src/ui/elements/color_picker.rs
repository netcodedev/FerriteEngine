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

struct Channel {
    value: DataSource<f32>,
    track: Plane,
    fill: Plane,
    knob: Plane,
    region: Region,
    color: (f32, f32, f32, f32),
    dragging: bool,
}

/// RGBA colour picker: four horizontal channel sliders in `[0, 1]` plus a
/// preview swatch showing the combined colour.
pub struct ColorPicker {
    handle: UIElementHandle,
    region: Region,
    channels: Vec<Channel>,
    swatch: Plane,
    frame: Plane,
}

impl ColorPicker {
    pub fn new(
        position: Position,
        width: f32,
        r: DataSource<f32>,
        g: DataSource<f32>,
        b: DataSource<f32>,
        a: DataSource<f32>,
    ) -> Self {
        let row_h = 14.0;
        let gap = 6.0;
        let padding = 8.0;
        let swatch_h = 28.0;
        let height = padding * 2.0 + swatch_h + gap + row_h * 4.0 + gap * 3.0;
        let size = Size::new(width, height);

        let frame = PlaneBuilder::new()
            .position(position)
            .size(size)
            .color((0.15, 0.15, 0.15, 1.0))
            .border_color((0.5, 0.5, 0.5, 1.0))
            .border_thickness(1.0)
            .border_radius_uniform(4.0)
            .build();
        let swatch = PlaneBuilder::new()
            .position(&position + (padding, padding, 1.0))
            .size(Size::new(width - padding * 2.0, swatch_h))
            .color((r.read(), g.read(), b.read(), a.read()))
            .border_color((0.4, 0.4, 0.4, 1.0))
            .border_thickness(1.0)
            .border_radius_uniform(3.0)
            .build();

        let colors = [
            (0.9, 0.2, 0.2, 1.0),
            (0.2, 0.8, 0.3, 1.0),
            (0.2, 0.4, 0.9, 1.0),
            (0.7, 0.7, 0.7, 1.0),
        ];
        let sources = [r, g, b, a];
        let channels = sources
            .into_iter()
            .enumerate()
            .map(|(i, src)| {
                let y = padding + swatch_h + gap + i as f32 * (row_h + gap);
                let row_pos = &position + (padding, y, 1.0);
                let row_size = Size::new(width - padding * 2.0, row_h);
                let track = PlaneBuilder::new()
                    .position(row_pos)
                    .size(row_size)
                    .color((0.1, 0.1, 0.1, 1.0))
                    .border_color((0.4, 0.4, 0.4, 1.0))
                    .border_thickness(1.0)
                    .border_radius_uniform(3.0)
                    .build();
                let fill = PlaneBuilder::new()
                    .position(&row_pos + (0.0, 0.0, 1.0))
                    .size(Size::new(0.0, row_h))
                    .color(colors[i])
                    .border_radius_uniform(3.0)
                    .build();
                let knob = PlaneBuilder::new()
                    .position(&row_pos + (0.0, -1.0, 2.0))
                    .size(Size::new(4.0, row_h + 2.0))
                    .color((0.95, 0.95, 0.95, 1.0))
                    .build();
                Channel {
                    value: src,
                    track,
                    fill,
                    knob,
                    region: Region::new(row_pos, row_size),
                    color: colors[i],
                    dragging: false,
                }
            })
            .collect();

        Self {
            handle: UIElementHandle::new(),
            region: Region::new(position, size),
            channels,
            swatch,
            frame,
        }
    }

    fn update_swatch(&mut self) {
        let r = self.channels[0].value.read().clamp(0.0, 1.0);
        let g = self.channels[1].value.read().clamp(0.0, 1.0);
        let b = self.channels[2].value.read().clamp(0.0, 1.0);
        let a = self.channels[3].value.read().clamp(0.0, 1.0);
        self.swatch.set_color((r, g, b, a));
    }
}

impl UIElement for ColorPicker {
    fn update(&mut self, _scene: &mut Scene) {
        for ch in &mut self.channels {
            let v = ch.value.read().clamp(0.0, 1.0);
            let w = ch.region.size.width * v;
            ch.fill.set_size(Size::new(w, ch.region.size.height));
            let kp = Position::new(
                ch.region.position.x + w - 2.0,
                ch.region.position.y - 1.0,
                ch.region.position.z + 2.0,
            );
            ch.knob.set_position(kp);
            // keep fill color in case someone edited it
            ch.fill.set_color(ch.color);
        }
        self.update_swatch();
    }

    fn render(&self) {
        self.frame.render();
        self.swatch.render();
        for ch in &self.channels {
            ch.track.render();
            ch.fill.render();
            ch.knob.render();
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
                let (x, y) = (x as f32, y as f32);
                for ch in &mut self.channels {
                    if ch.region.contains(x, y) {
                        ch.dragging = true;
                        let f = ((x - ch.region.position.x) / ch.region.size.width).clamp(0.0, 1.0);
                        ch.value.write(f);
                        return true;
                    }
                }
                false
            }
            MouseButton(MouseButtonLeft, Release, _) => {
                let mut was = false;
                for ch in &mut self.channels {
                    if ch.dragging {
                        was = true;
                    }
                    ch.dragging = false;
                }
                was
            }
            CursorPos(x, _y) => {
                let x = *x as f32;
                let mut handled = false;
                for ch in &mut self.channels {
                    if ch.dragging {
                        let f = ((x - ch.region.position.x) / ch.region.size.width).clamp(0.0, 1.0);
                        ch.value.write(f);
                        handled = true;
                    }
                }
                handled
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
        // reposition all children relative to new base
        let base = &self.region.position + &self.region.offset;
        self.frame.set_position(base);
        let row_h = 14.0;
        let gap = 6.0;
        let padding = 8.0;
        let swatch_h = 28.0;
        self.swatch.set_position(&base + (padding, padding, 1.0));
        for (i, ch) in self.channels.iter_mut().enumerate() {
            let y = padding + swatch_h + gap + i as f32 * (row_h + gap);
            let row_pos = &base + (padding, y, 1.0);
            ch.region.position = row_pos;
            ch.track.set_position(row_pos);
            ch.fill.set_position(&row_pos + (0.0, 0.0, 1.0));
        }
    }
    fn get_size(&self) -> &Size {
        &self.region.size
    }
    fn set_z_index(&mut self, z_index: f32) {
        self.region.position.z = z_index;
        self.frame.set_z_index(z_index);
        self.swatch.set_z_index(z_index + 1.0);
        for ch in &mut self.channels {
            ch.track.set_z_index(z_index + 1.0);
            ch.fill.set_z_index(z_index + 2.0);
            ch.knob.set_z_index(z_index + 3.0);
        }
    }
}
