use core::panic;

use ferrite::core::{
    entity::EntityHandle,
    renderer::ui::{
        panel::Panel,
        primitives::{Offset, Size, UIElementHandle},
        UIElement, UI,
    },
    scene::Scene,
};
use glfw::{Glfw, Window, WindowEvent};

pub struct EntityUI {
    panel: Panel,
}

impl UIElement for EntityUI {
    fn render(&mut self, scene: &mut Scene) {
        self.panel.render(scene);
    }

    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut Window,
        glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        self.panel.handle_events(scene, window, glfw, event)
    }

    fn add_children(&mut self, children: Vec<(Option<UIElementHandle>, Box<dyn UIElement>)>) {
        self.panel.add_children(children);
    }

    fn add_child_to(
        &mut self,
        parent: UIElementHandle,
        id: Option<UIElementHandle>,
        element: Box<dyn UIElement>,
    ) {
        self.panel.add_child_to(parent, id, element);
    }

    fn contains_child(&self, handle: &UIElementHandle) -> bool {
        self.panel.contains_child(handle)
    }

    fn get_offset(&self) -> Offset {
        self.panel.get_offset()
    }

    fn set_offset(&mut self, offset: Offset) {
        self.panel.set_offset(offset);
    }

    fn get_size(&self) -> Size {
        self.panel.get_size()
    }
}

impl EntityUI {
    pub fn new(scene: &Scene, entity_handle: EntityHandle) -> Self {
        let entity = scene.get_entity(&entity_handle);
        match entity {
            Some(entity) => {
                let input = UI::input(entity.get_name_ref(), |builder| builder.size(180.0, 20.0));
                let panel = UI::collapsible_dyn(entity.get_name_ref(), |builder| {
                    builder
                        .size(180.0, 40.0)
                        .closed()
                        .movable(false)
                        .add_child(None, input)
                });
                return Self { panel: *panel };
            }
            None => {
                panic!("Entity with handle {:?} not found", entity_handle);
            }
        }
    }
}
