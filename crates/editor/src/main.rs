use ferrite::core::{
    application::{Application, Layer},
    entity::Entity,
    renderer::ui::{primitives::UIElementHandle, UIRenderer, UI},
    scene::Scene,
    window::Window,
};
use glfw::{Glfw, WindowEvent};

mod ui;

use ui::entity::EntityUI;

fn main() {
    let mut application = Application::new(1280, 720, "Ferrite Editor");
    application.add_layer(Box::new(EditorLayer::new()));
    application.start();
}

struct EditorLayer {
    scene: Scene,
    ui: UIRenderer,

    entity_container: Option<UIElementHandle>,
}

impl EditorLayer {
    fn new() -> Self {
        Self {
            scene: Scene::new(),
            ui: UIRenderer::new(),
            entity_container: None,
        }
    }

    fn update_entity_ui_elements(&mut self) {
        let entities = self.scene.get_entities();
        for entity in entities {
            let handle = UIElementHandle::from(entity.id.into());
            if !self.ui.contains_key(&handle) {
                self.ui.insert_to(
                    self.entity_container.unwrap(),
                    Some(handle),
                    Box::new(EntityUI::new(&self.scene, entity.id)),
                );
            }
        }
    }
}

impl Layer for EditorLayer {
    fn on_attach(&mut self) {
        let handles = [UIElementHandle::new(), UIElementHandle::new()];
        let (controls_handle, entities_handle) = (
            std::cmp::min(handles[0], handles[1]),
            std::cmp::max(handles[0], handles[1]),
        );
        let controls = UI::container(|builder| {
            builder.size(200.0, 0.0).add_child(
                None,
                UI::button(
                    "Add Entity",
                    Box::new(move |scene| {
                        scene.add_entity(Entity::new("Entity"));
                    }),
                    |builder| builder,
                ),
            )
        });
        let entities = UI::container(|b| b);
        self.ui.add(UI::panel("Entities", move |builder| {
            builder
                .size(200.0, 200.0)
                .add_child(Some(controls_handle), controls)
                .add_child(Some(entities_handle), entities)
        }));
        self.entity_container = Some(entities_handle);
    }

    fn on_update(&mut self, window: &Window, delta_time: f64) {
        self.scene.update(delta_time);
        self.scene.render(window);

        self.update_entity_ui_elements();
        self.ui.render(&mut self.scene);
    }

    fn on_event(&mut self, glfw: &mut Glfw, window: &mut glfw::Window, event: &WindowEvent) {
        if self.ui.handle_events(&mut self.scene, window, glfw, &event) {
            return;
        }
        self.scene.handle_event(glfw, window, event);
    }

    fn get_name(&self) -> &str {
        "Editor"
    }
}
