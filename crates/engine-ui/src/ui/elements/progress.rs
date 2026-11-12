use crate::ui::theme;
use ferrite::core::{
    primitives::{Offset, Position, Region, Size},
    renderer::plane::{Plane, PlaneBuilder},
    scene::Scene,
    utils::DataSource,
};
use glfw::{Glfw, Window, WindowEvent};

use crate::ui::{element_handle::UIElementHandle, UIElement};

/// Read-only progress bar. `value` is expected in `[0, 1]`.
pub struct ProgressBar {
    handle: UIElementHandle,
    region: Region,
    value: DataSource<f32>,
    track: Plane,
    fill: Plane,
}

impl ProgressBar {
    pub fn new(position: Position, size: Size, value: DataSource<f32>) -> Self {
        let track = PlaneBuilder::new()
            .position(position)
            .size(size)
            .color(theme::BG_FIELD)
            .border_color(theme::BORDER)
            .border_thickness(1.0)
            .border_radius_uniform(size.height * 0.5)
            .build();
        let fill = PlaneBuilder::new()
            .position(&position + (0.0, 0.0, 1.0))
            .size(Size::new(0.0, size.height))
            .color(theme::ACCENT)
            .border_radius_uniform(size.height * 0.5)
            .build();
        Self {
            handle: UIElementHandle::new(),
            region: Region::new(position, size),
            value,
            track,
            fill,
        }
    }

    fn sync(&mut self) {
        let f = self.value.read().clamp(0.0, 1.0);
        let base = &self.region.position + &self.region.offset;
        self.track.set_position(base);
        self.fill.set_position(&base + (0.0, 0.0, 1.0));
        self.fill
            .set_size(Size::new(self.region.size.width * f, self.region.size.height));
    }
}

impl UIElement for ProgressBar {
    fn update(&mut self, _scene: &mut Scene) {
        self.sync();
    }

    fn render(&self) {
        self.track.render();
        self.fill.render();
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
        self.sync();
    }
    fn get_size(&self) -> &Size {
        &self.region.size
    }
    fn set_z_index(&mut self, z_index: f32) {
        self.region.position.z = z_index;
        self.track.set_z_index(z_index);
        self.fill.set_z_index(z_index + 1.0);
    }
}
