use ferrite::core::{
    primitives::{Offset, Position, Size},
    renderer::plane::{PlaneBuilder, PlaneRenderer},
    scene::Scene,
    utils::DataSource,
};

use crate::ui::{element_handle::UIElementHandle, elements::panel::Panel, UIElement, UI};

use super::Popup;

impl Popup {
    pub fn new(
        title: String,
        close_ref: DataSource<bool>,
        children: Vec<Box<dyn UIElement>>,
    ) -> Self {
        let mut panel = Panel::new(
            title,
            Position {
                x: 400.0,
                y: 300.0,
                z: 51.0,
            },
            Size {
                width: 200.0,
                height: 150.0,
            },
        );
        let mut button = UI::button(
            "x",
            Box::new(move |_| {
                close_ref.write(false);
            }),
        );
        button.set_size(Size {
            width: 18.0,
            height: 18.0,
        });
        panel.add_control(button);
        for child in children {
            panel.add_child(child);
        }
        let background = PlaneBuilder::new()
            .position((0.0, 0.0, 50.0).into())
            .size((5000.0, 5000.0).into())
            .color((0.0, 0.0, 0.0, 0.6))
            .build();
        Self {
            handle: UIElementHandle::new(),
            background,
            panel,
        }
    }
}

impl UIElement for Popup {
    fn update(&mut self, scene: &mut Scene) {
        self.panel.update(scene);
    }

    fn render(&self) {
        PlaneRenderer::render(&self.background);
        self.panel.render();
    }

    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut glfw::Window,
        glfw: &mut glfw::Glfw,
        event: &glfw::WindowEvent,
    ) -> bool {
        self.panel.handle_events(scene, window, glfw, event);
        true
    }

    fn add_child(&mut self, child: Box<dyn UIElement>) {
        self.panel.add_child(child);
    }

    fn add_child_to(&mut self, parent: UIElementHandle, element: Box<dyn UIElement>) {
        self.panel.add_child_to(parent, element);
    }

    fn contains_child(&self, handle: &UIElementHandle) -> bool {
        self.panel.contains_child(handle)
    }

    fn get_child(&self, handle: &UIElementHandle) -> Option<&Box<dyn UIElement>> {
        self.panel.get_child(handle)
    }

    fn get_child_mut(&mut self, handle: &UIElementHandle) -> Option<&mut Box<dyn UIElement>> {
        self.panel.get_child_mut(handle)
    }

    fn get_handle(&self) -> &UIElementHandle {
        &self.handle
    }

    fn get_offset(&self) -> &Offset {
        self.panel.get_offset()
    }

    fn set_offset(&mut self, offset: Offset) {
        self.panel.set_offset(offset);
    }

    fn get_size(&self) -> &Size {
        self.panel.get_size()
    }

    fn set_z_index(&mut self, z_index: f32) {
        self.panel.set_z_index(z_index);
    }
}
