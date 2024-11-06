use cgmath::{Point3, Vector3, Zero};
use glfw::{Action, Glfw, Key, WindowEvent};

use crate::core::{entity::{component::{camera_component::CameraComponent, model_component::ModelComponent, Component}, Entity}, model::ModelBuilder, scene::Scene};

use super::{Player, PlayerController};

impl Player {
    pub fn new<P: Into<Point3<f32>>>(position: P) -> Result<Entity, Box<dyn std::error::Error>> {
        let mut entity = Entity::new();
        entity.set_position(position);

        let mut model = ModelBuilder::new("Mannequin.fbx")?
            .with_animation("idle", "Idle.fbx")
            .with_animation("walk", "Walk.fbx")
            .with_animation("run", "Run.fbx")
            .with_animation("back", "Walk_Backwards.fbx")
            .with_animation("left", "Walk_Left.fbx")
            .with_animation("right", "Walk_Right.fbx")
            .build();
        model.init();
        model.blend_animations("walk", "run", 0.5, true);
        model.play_animation("idle");

        entity.add_component(ModelComponent::new(model));
        entity.add_component(PlayerController::new());

        Ok(entity)
    }
}

impl PlayerController {
    pub fn new() -> Self {
        Self {
            forward: 0.0,
            backward: 0.0,
            left: 0.0,
            right: 0.0,
            dirty: false,
        }
    }
}

impl Component for PlayerController {
    fn update(&mut self, scene: &mut Scene, entity: &mut Entity, _: f64) {
        let mut position_delta: Vector3<f32> = Vector3::zero();
        if let Some(model_component) = entity.get_component_mut::<ModelComponent>(){
            let model = model_component.get_model_mut();
            position_delta = model.reset_position();
            if self.dirty {
                let x_anim = if self.forward - self.backward > 0.0 {
                    "walk"
                } else if self.forward - self.backward < 0.0 {
                    "back"
                } else {
                    "idle"
                };
                let y_anim = if self.left - self.right > 0.0 {
                    "left"
                } else if self.left - self.right < 0.0 {
                    "right"
                } else {
                    "idle"
                };
                let blend = x_anim != "idle" && y_anim != "idle";
                if blend {
                    model.blend_animations(&x_anim, &y_anim, 0.5, true);
                } else {
                    if x_anim != "idle" {
                        model.play_animation(x_anim);
                    } else if y_anim != "idle" {
                        model.play_animation(y_anim);
                    } else {
                        model.play_animation("idle");
                    }
                }
            }
        }
        entity.set_position(entity.get_position() + position_delta);
        let new_pos = entity.get_position() - Vector3::new(0.1, -3.4, 3.0);
        let camera = scene.get_component_mut::<CameraComponent>().unwrap().get_camera_mut();
        camera.update(new_pos, camera.get_yaw(), camera.get_pitch());
        self.dirty = false;
    }

    fn handle_event(&mut self, _: &mut Glfw, _: &mut glfw::Window, event: &WindowEvent) {
        match event {
            glfw::WindowEvent::Key(Key::W, _, action, _) => {
                match action {
                    &Action::Press => {self.forward = 1.0; self.dirty = true;},
                    &Action::Release => {self.forward = 0.0; self.dirty = true;},
                    _ => {},
                }
            },
            glfw::WindowEvent::Key(Key::S, _, action, _) => {
                match action {
                    &Action::Press => {self.backward = 1.0; self.dirty = true;},
                    &Action::Release => {self.backward = 0.0; self.dirty = true;},
                    _ => {},
                }
            },
            glfw::WindowEvent::Key(Key::A, _, action, _) => {
                match action {
                    &Action::Press => {self.left = 1.0; self.dirty = true;},
                    &Action::Release => {self.left = 0.0; self.dirty = true;},
                    _ => {},
                }
            },
            glfw::WindowEvent::Key(Key::D, _, action, _) => {
                match action {
                    &Action::Press => {self.right = 1.0; self.dirty = true;},
                    &Action::Release => {self.right = 0.0; self.dirty = true;},
                    _ => {},
                }
            },
            _ => {},  
        }
    }
}