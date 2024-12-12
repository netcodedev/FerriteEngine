use core::panic;

use ferrite::core::{
    entity::{Entity, EntityHandle},
    renderer::ui::{
        primitives::{Offset, Size, UIElementHandle},
        UIElement, UI,
    },
    scene::Scene,
    utils::DataSource,
};
use glfw::{Glfw, Window, WindowEvent};

use super::{AddEntityButton, EditEntityButton, EntityUI};

impl UIElement for EntityUI {
    fn render(&mut self, scene: &mut Scene) {
        self.panel
            .add_children(match scene.get_entity(&self.entity_handle) {
                Some(entity) => entity
                    .get_children()
                    .iter()
                    .filter(|child| {
                        !self
                            .panel
                            .contains_child(&UIElementHandle::from(child.id.into()))
                    })
                    .map(|child| {
                        (
                            Some(UIElementHandle::from(child.id.into())),
                            Box::new(EntityUI::new(scene, child.id, self.get_size().width - 5.0))
                                as Box<dyn UIElement>,
                        )
                    })
                    .collect(),
                None => vec![],
            });
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

impl EntityUI {
    pub fn new(scene: &Scene, entity_handle: EntityHandle, width: f32) -> Self {
        let entity = scene.get_entity(&entity_handle);
        match entity {
            Some(entity) => {
                let name_ref = entity.get_name_ref();
                let mut panel = UI::collapsible_dyn(name_ref.clone(), move |builder| {
                    builder
                        .size(width, 40.0)
                        .closed()
                        .movable(false)
                        .add_control(
                            Some(UIElementHandle::from(0)),
                            Box::new(EditEntityButton::new(name_ref)) as Box<dyn UIElement>,
                        )
                        .add_control(
                            Some(UIElementHandle::from(1)),
                            Box::new(AddEntityButton::new(Some(entity_handle.clone())))
                                as Box<dyn UIElement>,
                        )
                        .with_end_gap(false)
                });
                panel.add_children(
                    entity
                        .get_children()
                        .iter()
                        .map(|child| {
                            (
                                Some(UIElementHandle::from(child.id.into())),
                                Box::new(EntityUI::new(scene, child.id, width))
                                    as Box<dyn UIElement>,
                            )
                        })
                        .collect(),
                );
                return Self {
                    panel: *panel,
                    entity_handle,
                };
            }
            None => {
                panic!("Entity with handle {:?} not found", entity_handle);
            }
        }
    }
}

impl AddEntityButton {
    pub fn new(entity_handle: Option<EntityHandle>) -> Self {
        Self {
            button: UI::button(
                "+",
                Box::new(move |scene| match entity_handle {
                    Some(entity_handle) => {
                        if let Some(entity) = scene.get_entity_mut(&entity_handle) {
                            entity.add_child(Entity::new("Entity"))
                        }
                    }
                    None => {
                        scene.add_entity(Entity::new("Entity"));
                    }
                }),
                |builder| builder.size(18.0, 18.0),
            ),
        }
    }
}

impl UIElement for AddEntityButton {
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

impl EditEntityButton {
    pub fn new(entity_name_ref: DataSource<String>) -> Self {
        let show_popup = DataSource::new(false);
        let clone = show_popup.clone();
        Self {
            button: UI::button(
                "Edit",
                Box::new(move |_| {
                    clone.write(true);
                }),
                |builder| builder.size(40.0, 18.0),
            ),
            show_popup: show_popup.clone(),
            popup: *UI::popup(
                "Edit Entity",
                show_popup.clone(),
                vec![
                    (
                        Some(UIElementHandle::from(1)),
                        UI::text("Edit Entity name", 16.0, |b| b),
                    ),
                    (
                        Some(UIElementHandle::from(2)),
                        UI::input(entity_name_ref, |b| b.size(200.0, 20.0)),
                    ),
                ],
            ),
        }
    }
}

impl UIElement for EditEntityButton {
    fn render(&mut self, scene: &mut Scene) {
        self.button.render(scene);
        if self.show_popup.read() {
            self.popup.render(scene);
        }
    }

    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut Window,
        glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        self.button.handle_events(scene, window, glfw, event)
            || (self.show_popup.read() && self.popup.handle_events(scene, window, glfw, event))
    }

    fn add_children(&mut self, _: Vec<(Option<UIElementHandle>, Box<dyn UIElement>)>) {
        panic!("EditEntityButton cannot have children");
    }

    fn add_child_to(
        &mut self,
        _: UIElementHandle,
        _: Option<UIElementHandle>,
        _: Box<dyn UIElement>,
    ) {
        panic!("EditEntityButton cannot have children");
    }

    fn contains_child(&self, _: &UIElementHandle) -> bool {
        false
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
