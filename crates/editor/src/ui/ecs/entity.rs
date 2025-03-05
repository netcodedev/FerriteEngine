use core::panic;

use ferrite::core::{
    entity::{Entity, EntityHandle},
    primitives::{Offset, Position, Size},
    scene::Scene,
    utils::DataSource,
};
use ferrite_ui::ui::{element_handle::UIElementHandle, UIElement, UI};
use glfw::{Glfw, Window, WindowEvent};

use super::{AddEntityButton, EditEntityButton, EntityUI};

impl EntityUI {
    pub fn new(scene: &Scene, entity_handle: EntityHandle, width: f32) -> Self {
        let entity = scene.get_entity(&entity_handle);
        match entity {
            Some(entity) => {
                let name_ref = entity.get_name_ref();
                let mut panel = UI::panel(
                    &name_ref.read(),
                    Position::default(),
                    Size {
                        width,
                        height: 40.0,
                    },
                );
                panel.close();
                panel.set_movable(false);
                panel.set_with_end_gap(false);
                panel.add_control({
                    let mut button = EditEntityButton::new(name_ref.clone());
                    button.set_handle(UIElementHandle::from(2));
                    Box::new(button)
                });
                panel.add_control({
                    let mut button = AddEntityButton::new(Some(entity_handle.clone()));
                    button.set_handle(UIElementHandle::from(3));
                    Box::new(button)
                });
                entity
                    .get_children()
                    .iter()
                    .map(|child| {
                        let mut entity_ui = EntityUI::new(scene, child.id, width);
                        entity_ui.set_handle(UIElementHandle::from(child.id.into()));
                        Box::new(entity_ui)
                    })
                    .for_each(|entity_ui| {
                        panel.add_child(entity_ui);
                    });
                return Self {
                    handle: UIElementHandle::from(entity_handle.into()),
                    panel: *panel,
                    entity_handle,
                };
            }
            None => {
                panic!("Entity with handle {:?} not found", entity_handle);
            }
        }
    }

    pub fn set_handle(&mut self, handle: UIElementHandle) {
        self.handle = handle;
    }
}

impl UIElement for EntityUI {
    fn update(&mut self, scene: &mut Scene) {
        if let Some(entity) = scene.get_entity(&self.entity_handle) {
            let elements = entity
                .get_children()
                .iter()
                .filter(|child| {
                    !self
                        .panel
                        .contains_child(&UIElementHandle::from(child.id.into()))
                })
                .map(|child| {
                    Box::new(EntityUI::new(scene, child.id, self.get_size().width - 5.0))
                })
                .collect::<Vec<Box<EntityUI>>>();
            elements.into_iter().for_each(|element| {
                self.panel.add_child(element);
            });
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

    fn add_child(&mut self, children: Box<dyn UIElement>) {
        self.panel.add_child(children);
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

impl AddEntityButton {
    pub fn new(entity_handle: Option<EntityHandle>) -> Self {
        let mut button = UI::button(
            "+",
            Box::new(move |scene| {
                match entity_handle {
                    Some(entity_handle) => {
                        if let Some(entity) = scene.get_entity_mut(&entity_handle) {
                            entity.add_child(Entity::new("Entity"))
                        }
                    }
                    None => {
                        scene.add_entity(Entity::new("Entity"));
                    }
                }
            }),
        );
        button.set_size(Size {
            width: 40.0,
            height: 18.0,
        });
        Self {
            handle: UIElementHandle::new(),
            button,
        }
    }

    pub fn set_handle(&mut self, handle: UIElementHandle) {
        self.handle = handle;
    }
}

impl UIElement for AddEntityButton {
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

    fn get_handle(&self) -> &UIElementHandle {
        &self.handle
    }
}

impl EditEntityButton {
    pub fn new(entity_name_ref: DataSource<String>) -> Self {
        let show_popup = DataSource::new(false);
        let clone = show_popup.clone();
        let mut button = UI::button(
            "Edit",
            Box::new(move |_| {
                clone.write(true);
            }),
        );
        button.set_size(Size {
            width: 40.0,
            height: 18.0,
        });
        let mut edit_entity = UI::text("Edit Entity name", 16.0);
        edit_entity.set_handle(UIElementHandle::from(4));
        let mut input = UI::input(entity_name_ref);
        input.set_size(Size {
            width: 200.0,
            height: 20.0,
        });
        input.set_handle(UIElementHandle::from(5));
        Self {
            handle: UIElementHandle::from(6),
            button,
            show_popup: show_popup.clone(),
            popup: *UI::popup(
                "Edit Entity",
                show_popup.clone(),
                vec![
                    edit_entity as Box<dyn UIElement>,
                    input as Box<dyn UIElement>,
                ],
            ),
        }
    }

    pub fn set_handle(&mut self, handle: UIElementHandle) {
        self.handle = handle;
    }
}

impl UIElement for EditEntityButton {
    fn update(&mut self, scene: &mut Scene) {
        self.button.update(scene);
        if self.show_popup.read() {
            self.popup.update(scene);
        }
    }

    fn render(&self) {
        self.button.render();
        if self.show_popup.read() {
            self.popup.render();
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

    fn add_child(&mut self, _: Box<dyn UIElement>) {
        panic!("EditEntityButton cannot have children");
    }

    fn add_child_to(&mut self, _: UIElementHandle, _: Box<dyn UIElement>) {
        panic!("EditEntityButton cannot have children");
    }

    fn contains_child(&self, _: &UIElementHandle) -> bool {
        false
    }

    fn get_child(&self, _handle: &UIElementHandle) -> Option<&Box<dyn UIElement>> {
        None
    }

    fn get_child_mut(&mut self, _handle: &UIElementHandle) -> Option<&mut Box<dyn UIElement>> {
        None
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

    fn get_handle(&self) -> &UIElementHandle {
        &self.handle
    }
}
