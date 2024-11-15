use cgmath::{Point3, Vector3, Zero};
use glfw::{Action, Glfw, Key, WindowEvent};

use crate::core::{
    entity::{
        component::{
            animation_component::AnimationComponent, camera_component::CameraComponent,
            model_component::ModelComponent, Component,
        },
        Entity,
    },
    model::{animation_graph::AnimationGraph, ModelBuilder},
    physics::rigidbody::RigidBody,
    scene::Scene,
};

use super::{Player, PlayerController};

impl Player {
    pub fn new<P: Into<Point3<f32>>>(
        scene: &mut Scene,
        position: P,
        animation_graph: AnimationGraph,
    ) -> Result<Entity, Box<dyn std::error::Error>> {
        let mut entity = Entity::new();
        entity.set_position(scene, position);

        let mut model = ModelBuilder::new("Mannequin.fbx")?.build();
        model.init();

        let animation_component = AnimationComponent::new(animation_graph);

        entity.add_component(animation_component);
        entity.add_component(RigidBody::new(scene, &entity));
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
        if let Some(animation_component) = entity.get_component_mut::<AnimationComponent>() {
            if self.dirty {
                animation_component.set_input("forward", self.forward);
                animation_component.set_input("backward", self.backward);
                animation_component.set_input("left", self.left);
                animation_component.set_input("right", self.right);
            }
        }
        if let Some(model_component) = entity.get_component_mut::<ModelComponent>() {
            let model = model_component.get_model_mut();
            position_delta += model.reset_position();
        }
        entity.set_position(scene, entity.get_position() + position_delta);
        let camera = scene
            .get_component_mut::<CameraComponent>()
            .unwrap()
            .get_camera_mut();
        camera.set_position(entity.get_position());
        self.dirty = false;
    }

    fn handle_event(&mut self, _: &mut Glfw, _: &mut glfw::Window, event: &WindowEvent) {
        match event {
            glfw::WindowEvent::Key(Key::W, _, action, _) => match action {
                &Action::Press => {
                    self.forward = 1.0;
                    self.dirty = true;
                }
                &Action::Release => {
                    self.forward = 0.0;
                    self.dirty = true;
                }
                _ => {}
            },
            glfw::WindowEvent::Key(Key::S, _, action, _) => match action {
                &Action::Press => {
                    self.backward = 1.0;
                    self.dirty = true;
                }
                &Action::Release => {
                    self.backward = 0.0;
                    self.dirty = true;
                }
                _ => {}
            },
            glfw::WindowEvent::Key(Key::A, _, action, _) => match action {
                &Action::Press => {
                    self.left = 1.0;
                    self.dirty = true;
                }
                &Action::Release => {
                    self.left = 0.0;
                    self.dirty = true;
                }
                _ => {}
            },
            glfw::WindowEvent::Key(Key::D, _, action, _) => match action {
                &Action::Press => {
                    self.right = 1.0;
                    self.dirty = true;
                }
                &Action::Release => {
                    self.right = 0.0;
                    self.dirty = true;
                }
                _ => {}
            },
            _ => {}
        }
    }
}
