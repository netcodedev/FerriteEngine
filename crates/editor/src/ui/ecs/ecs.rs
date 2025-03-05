use ferrite::core::{
    primitives::{Offset, Position, Size},
    scene::Scene,
};
use ferrite_ui::ui::{element_handle::UIElementHandle, UIElement, UI};
use glfw::{Glfw, Window, WindowEvent};

use super::{AddComponentButton, AddEntityButton, EntityComponentsPanel, EntityUI};

impl EntityComponentsPanel {
    pub fn new() -> Self {
        let mut panel = UI::panel(
            "Entities & Components",
            Position::default(),
            Size::new(300.0, 200.0),
        );
        let mut entity_panel = UI::panel("Entities", Position::default(), Size::new(290.0, 200.0));
        entity_panel.set_handle(UIElementHandle::from(0));
        entity_panel.set_movable(false);
        entity_panel.set_collapsible(true);
        entity_panel.add_control(Box::new(AddEntityButton::new(None)));
        let mut components_panel =
            UI::panel("Components", Position::default(), Size::new(290.0, 200.0));
        components_panel.set_handle(UIElementHandle::from(1));
        components_panel.set_movable(false);
        components_panel.set_collapsible(true);
        components_panel.add_control(Box::new(AddComponentButton::new(None)));
        panel.add_child(entity_panel);
        panel.add_child(components_panel);
        Self {
            handle: UIElementHandle::new(),
            panel,
            entity_panel_handle: UIElementHandle::from(0),
            components_panel_handle: UIElementHandle::from(1),
        }
    }
}

impl UIElement for EntityComponentsPanel {
    fn update(&mut self, scene: &mut Scene) {
        let entities = scene.get_entities();
        for entity in entities {
            let entity_handle = UIElementHandle::from(entity.id.into());
            if !self.panel.contains_child(&entity_handle) {
                let entity_ui = EntityUI::new(scene, entity.id, 280.0);
                self.panel.add_child_to(self.entity_panel_handle, Box::new(entity_ui));
            }
        }

        self.panel.update(scene);
    }
    fn render(&self) {
        self.panel.render();
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
        self.panel.set_offset(offset)
    }

    fn get_size(&self) -> &Size {
        self.panel.get_size()
    }

    fn set_z_index(&mut self, z_index: f32) {
        self.panel.set_z_index(z_index)
    }
}
