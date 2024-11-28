use glfw::{Glfw, WindowEvent};

use crate::core::{
    renderer::{
        plane::{PlaneBuilder, PlaneRenderer},
        text::{Fonts, Text},
        ui::{container::ContainerBuilder, UIElement, UIElementHandle},
    },
    scene::Scene,
};

use super::Collapsible;

impl Collapsible {
    pub fn new(title: &str, position: (f32, f32, f32), size: (f32, f32)) -> Self {
        let content = ContainerBuilder::new()
            .position(position.0, position.1 + 20.0)
            .size(size.0, size.1 - 20.0)
            .build();
        let plane = PlaneBuilder::new()
            .position(position)
            .size((size.0, size.1))
            .color((0.2, 0.2, 0.2, 1.0))
            .border_radius_uniform(5.0)
            .border_thickness(1.0)
            .build();
        let header_plane = PlaneBuilder::new()
            .position(position)
            .size((size.0, 20.0))
            .color((0.3, 0.3, 0.3, 1.0))
            .border_radius_uniform(5.0)
            .border_thickness(1.0)
            .border_color((0.7, 0.7, 0.7, 1.0))
            .build();
        Self {
            is_open: false,
            title: title.to_string(),
            content,
            offset: (0.0, 0.0),
            position,
            size,
            text: Text::new(Fonts::RobotoMono, 0, 0, 16.0, title.to_string()),
            plane,
            header_plane,
        }
    }
}

impl UIElement for Collapsible {
    fn render(&mut self, scene: &mut Scene) {
        PlaneRenderer::render(&self.plane);
        PlaneRenderer::render(&self.header_plane);
        self.text.set_content(self.title.clone());
        self.text.render_at(
            (self.offset.0 + self.position.0 + 8.0) as i32,
            (self.offset.1 + self.position.1 + 2.0) as i32,
        );
        if self.is_open {
            self.content.render(scene);
        }
    }

    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut glfw::Window,
        glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        self.content.handle_events(scene, window, glfw, event)
    }

    fn add_children(
        &mut self,
        children: Vec<(
            Option<crate::core::renderer::ui::UIElementHandle>,
            Box<dyn UIElement>,
        )>,
    ) {
        self.content.add_children(children);
    }

    fn contains_child(&self, handle: &UIElementHandle) -> bool {
        self.content.contains_child(handle)
    }

    fn get_offset(&self) -> (f32, f32) {
        self.offset
    }

    fn set_offset(&mut self, offset: (f32, f32)) {
        self.offset = offset;
        self.plane.set_position((
            self.position.0 + self.offset.0,
            self.position.1 + self.offset.1,
            0.0,
        ));
        self.header_plane.set_position((
            self.position.0 + self.offset.0,
            self.position.1 + self.offset.1,
            0.0,
        ));
        self.content.set_offset((
            self.offset.0 + self.position.0,
            self.offset.1 + self.position.1 + 20.0,
        ));
    }

    fn get_size(&self) -> (f32, f32) {
        self.size
    }
}
