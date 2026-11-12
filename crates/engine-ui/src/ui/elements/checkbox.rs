use crate::ui::theme;
use ferrite::core::{
    primitives::{Offset, Position, Region, Size},
    renderer::{
        plane::{Plane, PlaneBuilder},
        text::{icons, Text},
    },
    scene::Scene,
    utils::DataSource,
};
use glfw::{
    Action::Press, Glfw, MouseButtonLeft, Window,
    WindowEvent::{self, CursorPos, MouseButton},
};

use crate::ui::{element_handle::UIElementHandle, UIElement};

pub struct Checkbox {
    handle: UIElementHandle,
    region: Region,
    value: DataSource<bool>,
    is_hovering: bool,
    box_plane: Plane,
    check_plane: Plane,
    check_icon: Text,
}

impl Checkbox {
    pub fn new(position: Position, value: DataSource<bool>) -> Self {
        let size = Size::new(18.0, 18.0);
        let box_plane = PlaneBuilder::new()
            .position(position)
            .size(size)
            .color(theme::BG_FIELD)
            .border_color(theme::BORDER_STRONG)
            .border_thickness(1.0)
            .border_radius_uniform(theme::RADIUS)
            .build();
        let check_plane = PlaneBuilder::new()
            .position(&position + (0.0, 0.0, 1.0))
            .size(size)
            .color(theme::ACCENT)
            .border_radius_uniform(theme::RADIUS)
            .build();
        Self {
            handle: UIElementHandle::new(),
            region: Region::new(position, size),
            value,
            is_hovering: false,
            box_plane,
            check_plane,
            check_icon: theme::icon(icons::CHECK_MARK, 14.0, (1.0, 1.0, 1.0, 1.0)),
        }
    }
}

impl UIElement for Checkbox {
    fn update(&mut self, _scene: &mut Scene) {
        let pos = &self.region.position + &self.region.offset;
        let x = ((self.region.size.width - self.check_icon.measure_width()) * 0.5).round();
        let y = theme::center_y(&self.check_icon, self.region.size.height);
        self.check_icon
            .prepare_render_at(&pos + (x, y, 2.0));
    }

    fn render(&self) {
        self.box_plane.render();
        if self.value.read() {
            self.check_plane.render();
            self.check_icon.render();
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
                    let v = self.value.read();
                    self.value.write(!v);
                    return true;
                }
                false
            }
            CursorPos(x, y) => {
                let inside = self.region.contains(*x as f32, *y as f32);
                if inside && !self.is_hovering {
                    window.set_cursor(Some(glfw::Cursor::standard(glfw::StandardCursor::Hand)));
                    self.is_hovering = true;
                    self.box_plane.set_color(theme::BG_CONTROL);
                } else if !inside && self.is_hovering {
                    window.set_cursor(None);
                    self.is_hovering = false;
                    self.box_plane.set_color(theme::BG_FIELD);
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
        self.box_plane.set_position(pos);
        self.check_plane.set_position(&pos + (0.0, 0.0, 1.0));
    }
    fn get_size(&self) -> &Size {
        &self.region.size
    }
    fn set_z_index(&mut self, z_index: f32) {
        self.region.position.z = z_index;
        self.box_plane.set_z_index(z_index);
        self.check_plane.set_z_index(z_index + 1.0);
        self.check_icon.set_z_index(z_index + 2.0);
    }
}
