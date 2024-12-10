use ferrite::core::{
    application::{Application, Layer},
    renderer::ui::{primitives::UIElementHandle, UIRenderer, UI},
    scene::Scene,
    window::Window,
};
use glfw::{Glfw, WindowEvent};
use ui::entity::{AddEntityButton, EntityUI};

mod ui;

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
                    Box::new(EntityUI::new(&self.scene, entity.id, 280.0)),
                );
            }
        }
    }
}

impl Layer for EditorLayer {
    fn on_attach(&mut self) {
        let entities_handle = UIElementHandle::new();
        let entities = UI::container(|b| b);
        self.ui.add(UI::panel("Entities", move |builder| {
            builder
                .size(300.0, 200.0)
                .add_control(None, Box::new(AddEntityButton::new(None)))
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
