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
        component::{camera_component::CameraComponent, debug_component::DebugController},
        Entity,
    },
    model::{
        animation_graph::{AnimationGraph, State},
        Animation,
    },
    renderer::{
        light::skylight::SkyLight,
        ui::{UIRenderer, UI},
    },
    scene::Scene,
    window::Window,
};
use std::error::Error;
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
    pub fn new(width: u32, height: u32) -> Result<WorldLayer, Box<dyn Error>> {
        let mut scene = Scene::new();
        scene.add_shadow_map(4096, 4096);
        let mut camera = Camera::new((0.0, 0.0, 0.0), Deg(-263.0), Deg(-30.0));
        camera.set_relative_position((0.25, 1.33, -2.05));
        let projection: Projection = Projection::new(width, height, Deg(45.0), 0.1, 100.0);
        let camera_controller = CameraController::new(10.0, 1.0);
        let mut entity = Entity::new();
        entity.add_component(CameraComponent::new(camera, projection, camera_controller));
        scene.add_entity(entity);

        let mut skylight = Entity::new();
        skylight.add_component(SkyLight::new((10.0, 600.0, 10.0)));
        scene.add_entity(skylight);

        let ui = UIRenderer::new();

        let mut terrain_entity = Entity::new();
        terrain_entity.add_component(Terrain::<DualContouringChunk>::new(2));
        terrain_entity.add_child(Player::new(
            &mut scene,
            (0.0, 55.0, 0.0),
            create_animation_graph()?,
        )?);

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
                            if let Some(camera_component) = scene.get_component::<CameraComponent>()
                            {
                                camera_component
                                    .get_camera_controller()
                                    .get_speed()
                                    .to_string()
                            } else {
                                "".to_string()
                            }
                        })
                        .set_fn(move |scene, v| {
                            if let Some(camera_component) =
                                scene.get_component_mut::<CameraComponent>()
                            {
                                match v.parse::<f32>() {
                                    Ok(v) => {
                                        camera_component.get_camera_controller_mut().set_speed(v)
                                    }
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
        "World"
    }
}

fn create_animation_graph() -> Result<AnimationGraph, Box<dyn Error>> {
    // Animation Graph visualization
    //
    // +-----------------+      +-----------------+       +-----------------+
    // | forward_left    |------| walk            |-------| forward_right   |
    // +-----------------+      +-----------------+       +-----------------+
    //        |                        |                        |
    // +-----------------+      +-----------------+       +-----------------+
    // | left            |------| idle            |-------| right           |
    // +-----------------+      +-----------------+       +-----------------+
    //        |                        |                        |
    // +-----------------+      +-----------------+       +-----------------+
    // | backward_left   |------| back            |-------| backward_right  |
    // +-----------------+      +-----------------+       +-----------------+

    let mut animation_graph = AnimationGraph::new();
    animation_graph.add_input("forward", 0.0);
    animation_graph.add_input("backward", 0.0);
    animation_graph.add_input("left", 0.0);
    animation_graph.add_input("right", 0.0);

    let transition_speed = 0.5;

    let mut idle_state = State::new("idle");
    idle_state.add_animation(Animation::from_file("idle", "Idle.fbx")?);
    idle_state.add_transition(
        "walk",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] > 0.0 && inputs["left"] - inputs["right"] == 0.0
        }),
        transition_speed,
    );
    idle_state.add_transition(
        "back",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] < 0.0 && inputs["left"] - inputs["right"] == 0.0
        }),
        transition_speed,
    );
    idle_state.add_transition(
        "left",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] == 0.0 && inputs["left"] - inputs["right"] > 0.0
        }),
        transition_speed,
    );
    idle_state.add_transition(
        "right",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] == 0.0 && inputs["left"] - inputs["right"] < 0.0
        }),
        transition_speed,
    );
    animation_graph.set_default_state(idle_state);

    let mut walk_state = State::new("walk");
    walk_state.add_animation(Animation::from_file("walk", "Walk.fbx")?);
    walk_state.add_transition(
        "idle",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] == 0.0 && inputs["left"] - inputs["right"] == 0.0
        }),
        transition_speed,
    );
    walk_state.add_transition(
        "forward_left",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] > 0.0 && inputs["left"] - inputs["right"] > 0.0
        }),
        transition_speed,
    );
    walk_state.add_transition(
        "forward_right",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] > 0.0 && inputs["left"] - inputs["right"] < 0.0
        }),
        transition_speed,
    );
    animation_graph.add_state(walk_state);

    let mut run_state = State::new("run");
    run_state.add_animation(Animation::from_file("run", "Run.fbx")?);
    animation_graph.add_state(run_state);

    let mut back_state = State::new("back");
    back_state.add_animation(Animation::from_file("back", "Walk_Backwards.fbx")?);
    back_state.add_transition(
        "idle",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] == 0.0 && inputs["left"] - inputs["right"] == 0.0
        }),
        transition_speed,
    );
    back_state.add_transition(
        "backward_left",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] < 0.0 && inputs["left"] - inputs["right"] > 0.0
        }),
        transition_speed,
    );
    back_state.add_transition(
        "backward_right",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] < 0.0 && inputs["left"] - inputs["right"] < 0.0
        }),
        transition_speed,
    );
    animation_graph.add_state(back_state);

    let mut left_state = State::new("left");
    left_state.add_animation(Animation::from_file("left", "Walk_Left.fbx")?);
    left_state.add_transition(
        "idle",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] == 0.0 && inputs["left"] - inputs["right"] == 0.0
        }),
        transition_speed,
    );
    left_state.add_transition(
        "forward_left",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] > 0.0 && inputs["left"] - inputs["right"] > 0.0
        }),
        transition_speed,
    );
    left_state.add_transition(
        "backward_left",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] < 0.0 && inputs["left"] - inputs["right"] > 0.0
        }),
        transition_speed,
    );
    animation_graph.add_state(left_state);

    let mut right_state = State::new("right");
    right_state.add_animation(Animation::from_file("right", "Walk_Right.fbx")?);
    right_state.add_transition(
        "idle",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] == 0.0 && inputs["left"] - inputs["right"] == 0.0
        }),
        transition_speed,
    );
    right_state.add_transition(
        "forward_right",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] > 0.0 && inputs["left"] - inputs["right"] < 0.0
        }),
        transition_speed,
    );
    right_state.add_transition(
        "backward_right",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] < 0.0 && inputs["left"] - inputs["right"] < 0.0
        }),
        transition_speed,
    );
    animation_graph.add_state(right_state);

    let mut forward_left_state = State::new("forward_left");
    forward_left_state.add_animation(Animation::from_file("walk", "Walk.fbx")?);
    forward_left_state.add_animation(Animation::from_file("left", "Walk_Left.fbx")?);
    forward_left_state.add_transition(
        "walk",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] > 0.0 && inputs["left"] - inputs["right"] == 0.0
        }),
        transition_speed,
    );
    forward_left_state.add_transition(
        "left",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] == 0.0 && inputs["left"] - inputs["right"] > 0.0
        }),
        transition_speed,
    );
    forward_left_state.sync_animations(true);
    animation_graph.add_state(forward_left_state);

    let mut forward_right_state = State::new("forward_right");
    forward_right_state.add_animation(Animation::from_file("walk", "Walk.fbx")?);
    forward_right_state.add_animation(Animation::from_file("right", "Walk_Right.fbx")?);
    forward_right_state.add_transition(
        "walk",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] > 0.0 && inputs["left"] - inputs["right"] == 0.0
        }),
        transition_speed,
    );
    forward_right_state.add_transition(
        "right",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] == 0.0 && inputs["left"] - inputs["right"] < 0.0
        }),
        transition_speed,
    );
    forward_right_state.sync_animations(true);
    animation_graph.add_state(forward_right_state);

    let mut backward_left_state = State::new("backward_left");
    backward_left_state.add_animation(Animation::from_file("back", "Walk_Backwards.fbx")?);
    backward_left_state.add_animation(Animation::from_file("left", "Walk_Left.fbx")?);
    backward_left_state.add_transition(
        "back",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] < 0.0 && inputs["left"] - inputs["right"] == 0.0
        }),
        transition_speed,
    );
    backward_left_state.add_transition(
        "left",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] == 0.0 && inputs["left"] - inputs["right"] > 0.0
        }),
        transition_speed,
    );
    backward_left_state.sync_animations(true);
    animation_graph.add_state(backward_left_state);

    let mut backward_right_state = State::new("backward_right");
    backward_right_state.add_animation(Animation::from_file("back", "Walk_Backwards.fbx")?);
    backward_right_state.add_animation(Animation::from_file("right", "Walk_Right.fbx")?);
    backward_right_state.add_transition(
        "back",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] < 0.0 && inputs["left"] - inputs["right"] == 0.0
        }),
        transition_speed,
    );
    backward_right_state.add_transition(
        "right",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] == 0.0 && inputs["left"] - inputs["right"] < 0.0
        }),
        transition_speed,
    );
    backward_right_state.sync_animations(true);
    animation_graph.add_state(backward_right_state);

    Ok(animation_graph)
}
