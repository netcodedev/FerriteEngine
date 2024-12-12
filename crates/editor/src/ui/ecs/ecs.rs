use ferrite::core::{
    renderer::ui::{
        primitives::{Offset, Size, UIElementHandle},
        UIElement, UI,
    },
    scene::Scene,
};

use super::{AddComponentButton, AddEntityButton, EntityComponentsPanel, EntityUI};

impl EntityComponentsPanel {
    pub fn new() -> Self {
        let mut panel = UI::panel("Entities & Components", |builder| {
            builder.size(300.0, 200.0)
        });
        let entity_panel = UI::panel("Entities", |builder| {
            builder
                .movable(false)
                .size(290.0, 200.0)
                .add_control(None, Box::new(AddEntityButton::new(None)))
        });
        let components_panel = UI::panel("Components", |builder| {
            builder
                .movable(false)
                .size(290.0, 200.0)
                .add_control(None, Box::new(AddComponentButton::new(None)))
        });
        panel.add_children(vec![
            (Some(UIElementHandle::from(0)), entity_panel),
            (Some(UIElementHandle::from(1)), components_panel),
        ]);
        Self {
            panel,
            entity_panel_handle: UIElementHandle::from(0),
            components_panel_handle: UIElementHandle::from(1),
        }
    }

    pub fn update(&mut self, scene: &mut Scene) {
        let entities = scene.get_entities();
        for entity in entities {
            let entity_handle = UIElementHandle::from(entity.id.into());
            if !self.panel.contains_child(&entity_handle) {
                self.panel.add_child_to(
                    self.entity_panel_handle,
                    Some(entity_handle),
                    Box::new(EntityUI::new(scene, entity.id, 280.0)),
                );
            }
        }
    }
}

impl UIElement for EntityComponentsPanel {
    fn render(&mut self, scene: &mut Scene) {
        self.update(scene);

        self.panel.render(scene);
    }

    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut glfw::Window,
        glfw: &mut glfw::Glfw,
        event: &glfw::WindowEvent,
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
