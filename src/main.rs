use cgmath::Deg;
use glfw::{Glfw, WindowEvent};
use player::Player;

mod core;
mod player;
mod terrain;
use core::{
    application::{Application, Layer},
    camera::{Camera, CameraController, Projection},
    entity::{
        component::{
            camera_component::CameraComponent, debug_component::DebugController,
        },
        Entity,
    },
    renderer::ui::{UIRenderer, UI},
    scene::Scene,
};
use terrain::{dual_contouring::DualContouringChunk, Terrain};

fn main() {
    let mut application = Application::new(1280, 720, "Engine");
    if let Ok(layer) = WorldLayer::new(1280, 720) {
        application.add_layer(Box::new(layer));
        application.start();
    }
}

struct WorldLayer {
    scene: Scene,
    ui: UIRenderer,
}

impl WorldLayer {
    pub fn new(width: u32, height: u32) -> Result<WorldLayer, Box<dyn std::error::Error>> {
        let mut scene = Scene::new();
        let camera = Camera::new((-121.1, 54.0, -35.0), Deg(-263.0), Deg(-30.0));
        let projection: Projection = Projection::new(width, height, Deg(45.0), 0.1, 100.0);
        let camera_controller = CameraController::new(10.0, 1.0);
        let mut entity = Entity::new();
        entity.add_component(CameraComponent::new(camera, projection, camera_controller));
        scene.add_entity(entity);
        let ui = UIRenderer::new();

        let mut terrain_entity = Entity::new();
        terrain_entity.add_component(Terrain::<DualContouringChunk>::new());
        terrain_entity.add_child(Player::new((-121.0, 50.6, -32.0))?);

        scene.add_entity(terrain_entity);

        let mut debug = Entity::new();
        debug.add_component(DebugController::new());
        scene.add_entity(debug);

        Ok(Self { scene, ui })
    }
}

impl Layer for WorldLayer {
    fn on_attach(&mut self) {
        self.ui.add(UI::panel("Camera controls", |builder| {
            builder
                .position(10.0, 130.0)
                .add_child(UI::text("Camera Speed", 16.0, |b| b))
                .add_child(UI::input(|input| {
                    input
                        .size(190.0, 26.0)
                        .get_fn(|scene| {
                            if let Some(camera_component) = scene.get_component::<CameraComponent>() {
                                camera_component.get_camera_controller().get_speed().to_string()
                            } else {
                                "".to_string()
                            }
                        })
                        .set_fn(move |scene, v| {
                            if let Some(camera_component) = scene.get_component_mut::<CameraComponent>() {
                                match v.parse::<f32>() {
                                    Ok(v) => camera_component.get_camera_controller_mut().set_speed(v),
                                    Err(_) => {}
                                }
                            }
                        })
                }))
                .add_child(UI::button(
                    "Reset Speed",
                    Box::new(move |scene| {
                        let camera_controller = scene
                            .get_component_mut::<CameraComponent>()
                            .unwrap()
                            .get_camera_controller_mut();
                        camera_controller.set_speed(10.0);
                    }),
                    |b| b,
                ))
        }));
    }

    fn on_update(&mut self, delta_time: f64) {
        self.scene.update(delta_time);
        self.scene.render();

        self.ui.render(&mut self.scene);
    }

    fn on_event(&mut self, glfw: &mut Glfw, window: &mut glfw::Window, event: &WindowEvent) {
        if self.ui.handle_events(&mut self.scene, window, glfw, &event) {
            return;
        }
        self.scene.handle_event(glfw, window, event);
    }

    fn get_name(&self) -> &str {
        "World"
    }
}
