use cgmath::Deg;
use ferrite_ui::ui::{UIElement, UI};
use glfw::{Glfw, WindowEvent};

use ferrite::{
    core::{
        application::{Application, Layer},
        camera::{Camera, CameraController, Projection},
        entity::{
            component::{camera_component::CameraComponent, debug_component::DebugController},
            Entity,
        },
        primitives::{Position, Size},
        renderer::light::skylight::SkyLight,
        scene::Scene,
        window::Window,
    },
    player::Player,
    terrain::{dual_contouring::DualContouringChunk, scatter::Scatter, Terrain},
};
use std::error::Error;

const TERRAIN_SEED: u64 = 2;

fn main() {
    let mut application = Application::new(1280, 720, "Engine");
    match WorldLayer::new(1280, 720) {
        Ok(layer) => {
            application.add_layer(Box::new(layer));
            application.start();
        }
        Err(e) => eprintln!("Failed to initialize world: {e}"),
    }
}

struct WorldLayer {
    scene: Scene,
    ui: UI,
}

impl WorldLayer {
    pub fn new(width: u32, height: u32) -> Result<WorldLayer, Box<dyn Error>> {
        let mut scene = Scene::new();
        scene.add_shadow_map(4096, 4096);
        scene.add_debug_maps(width, height);
        let camera = Camera::new((0.0, 0.0, 0.0), Deg(0.0), Deg(0.0));
        // FOV and far plane chosen to match the look the engine had under the
        // old projection: ~64° vertical FOV (vs nominal 45° before, which the
        // broken wgpu adjustment was silently widening) and a 1000m far plane
        // (the old matrix effectively extended the far clip to infinity).
        let projection: Projection = Projection::new(width, height, Deg(64.0), 0.1, 1000.0);
        let camera_controller = CameraController::new(10.0, 1.0);
        let mut entity = Entity::new("camera");
        entity.add_component(CameraComponent::new(camera, projection, camera_controller));
        scene.add_entity(entity);

        let mut skylight = Entity::new("skylight");
        skylight.add_component(SkyLight::new((30.0, 20.0, 10.0)));
        scene.add_entity(skylight);

        let ui = UI::new();

        let mut terrain_entity = Entity::new("terrain");
        terrain_entity.add_component(
            Terrain::<DualContouringChunk>::new(TERRAIN_SEED)
                .with_scatter(Scatter::default_layers()),
        );
        terrain_entity.add_child(Player::new(
            &mut scene,
            (0.0, 52.0, 0.0),
            ferrite::player::default_animation_graph()?,
        )?);

        scene.add_entity(terrain_entity);

        let mut debug = Entity::new("debug");
        debug.add_component(DebugController::new());
        scene.add_entity(debug);

        Ok(Self { scene, ui })
    }
}

impl Layer for WorldLayer {
    fn on_attach(&mut self) {
        let camera_speed_ref = self
            .scene
            .get_component::<CameraComponent>()
            .unwrap()
            .get_camera_controller()
            .get_speed_ref();
        let mut panel = UI::panel(
            "Camera controls",
            Position {
                x: 10.0,
                y: 130.0,
                z: 0.0,
            },
            Size::default(),
        );
        panel.add_child(UI::text("Camera Speed", 16.0));
        let mut input = UI::input(camera_speed_ref.clone());
        input.set_size(Size {
            width: 190.0,
            height: 26.0,
        });
        panel.add_child(input);
        panel.add_child(UI::button(
            "Reset Speed",
            Box::new(move |scene| {
                let camera_controller = scene
                    .get_component_mut::<CameraComponent>()
                    .unwrap()
                    .get_camera_controller_mut();
                camera_controller.set_speed(10.0);
            }),
        ));
        self.ui.add(panel);
    }

    fn on_update(&mut self, window: &Window, delta_time: f64) {
        self.scene.update(delta_time);
        self.scene.render(window);

        self.ui.update(&mut self.scene);
        self.ui.render();
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
