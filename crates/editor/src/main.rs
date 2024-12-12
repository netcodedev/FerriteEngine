mod ui;

use ferrite::core::{
    application::{Application, Layer},
    renderer::ui::{primitives::UIElementHandle, UIRenderer},
    scene::Scene,
    window::Window,
};
use glfw::{Glfw, WindowEvent};
use ui::ecs::EntityComponentsPanel;

fn main() {
    let mut application = Application::new(1280, 720, "Ferrite Editor");
    application.add_layer(Box::new(EditorLayer::new()));
    application.start();
}

struct EditorLayer {
    scene: Scene,
    ui: UIRenderer,

    ecs_panel_handle: UIElementHandle,
}

impl EditorLayer {
    fn new() -> Self {
        let mut ui = UIRenderer::new();
        let ecs_panel_handle = ui.add(Box::new(EntityComponentsPanel::new()));
        Self {
            scene: Scene::new(),
            ui,
            ecs_panel_handle,
        }
    }
}

impl Layer for EditorLayer {
    fn on_update(&mut self, window: &Window, delta_time: f64) {
        self.scene.update(delta_time);
        self.scene.render(window);

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
