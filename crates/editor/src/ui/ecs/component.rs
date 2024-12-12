use ferrite::core::{
    entity::EntityHandle,
    renderer::ui::{
        button::Button,
        primitives::{Offset, Size, UIElementHandle},
        UIElement, UI,
    },
    scene::Scene,
};
use glfw::{Glfw, Window, WindowEvent};

use super::AddComponentButton;

impl AddComponentButton {
    pub fn new(entity_handle: Option<EntityHandle>) -> Self {
        let on_click = Box::new(move |scene: &mut Scene| {
            if let Some(entity_handle) = entity_handle {
                // scene.add_component(entity_handle);
            }
        });
        Self {
            button: UI::button("+", on_click, |builder| builder.size(18.0, 18.0)),
        }
    }
}

impl UIElement for AddComponentButton {
    fn render(&mut self, scene: &mut Scene) {
        self.button.render(scene);
    }

    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut Window,
        glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        self.button.handle_events(scene, window, glfw, event)
    }

    fn add_children(&mut self, children: Vec<(Option<UIElementHandle>, Box<dyn UIElement>)>) {
        self.button.add_children(children);
    }

    fn add_child_to(
        &mut self,
        parent: UIElementHandle,
        id: Option<UIElementHandle>,
        element: Box<dyn UIElement>,
    ) {
        self.button.add_child_to(parent, id, element);
    }

    fn contains_child(&self, handle: &UIElementHandle) -> bool {
        self.button.contains_child(handle)
    }

    fn get_offset(&self) -> &Offset {
        self.button.get_offset()
    }

    fn set_offset(&mut self, offset: Offset) {
        self.button.set_offset(offset);
    }

    fn get_size(&self) -> &Size {
        self.button.get_size()
    }

    fn set_z_index(&mut self, z_index: f32) {
        self.button.set_z_index(z_index);
    }
}
