use ferrite::core::{
    entity::EntityHandle,
    primitives::{Offset, Size},
    scene::Scene,
};
use ferrite_ui::ui::{element_handle::UIElementHandle, UIElement, UI};
use glfw::{Glfw, Window, WindowEvent};

use super::AddComponentButton;

impl AddComponentButton {
    pub fn new(entity_handle: Option<EntityHandle>) -> Self {
        let on_click = Box::new(move |_scene: &mut Scene| {
            if let Some(_entity_handle) = entity_handle {
                // scene.add_component(entity_handle);
            }
        });
        let mut button = UI::button("+", on_click);
        button.set_size(Size {
            width: 18.0,
            height: 18.0,
        });
        Self {
            handle: UIElementHandle::new(),
            button,
        }
    }
}

impl UIElement for AddComponentButton {
    fn update(&mut self, scene: &mut Scene) {
        self.button.update(scene);
    }

    fn render(&self) {
        self.button.render();
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

    fn add_child(&mut self, child: Box<dyn UIElement>) {
        self.button.add_child(child);
    }

    fn add_child_to(&mut self, parent: UIElementHandle, element: Box<dyn UIElement>) {
        self.button.add_child_to(parent, element);
    }

    fn contains_child(&self, handle: &UIElementHandle) -> bool {
        self.button.contains_child(handle)
    }

    fn get_child(&self, handle: &UIElementHandle) -> Option<&Box<dyn UIElement>> {
        self.button.get_child(handle)
    }

    fn get_child_mut(&mut self, handle: &UIElementHandle) -> Option<&mut Box<dyn UIElement>> {
        self.button.get_child_mut(handle)
    }

    fn get_handle(&self) -> &UIElementHandle {
        &self.handle
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
