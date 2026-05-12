use cgmath::{Point3, Vector3, Zero};
use glfw::{Action, Glfw, Key, WindowEvent};
use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};

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

use super::{BoneColliders, Player, PlayerController};

// Capsule half-height and radius sized to sit flush with the ground.
// The physics center is at entity position; capsule bottom = center - HALF_HEIGHT - RADIUS.
// These values match the original ball(1.0) footprint so spawn height (Y=52) stays correct.
const CAPSULE_HALF_HEIGHT: f32 = 0.5;
const CAPSULE_RADIUS: f32 = 0.5;

impl Player {
    pub fn new<P: Into<Point3<f32>>>(
        scene: &mut Scene,
        position: P,
        animation_graph: AnimationGraph,
    ) -> Result<Entity, Box<dyn std::error::Error>> {
        let mut entity = Entity::new("player");
        entity.set_position(scene, position);

        let mut model = ModelBuilder::new("Mannequin.fbx")?.build();
        model.init();

        let animation_component = AnimationComponent::new(animation_graph);

        let collider = ColliderBuilder::capsule_y(CAPSULE_HALF_HEIGHT, CAPSULE_RADIUS)
            .friction(0.0)
            .build();

        let translation = entity.get_position();
        let rigid_body = RigidBodyBuilder::dynamic()
            .translation(rapier3d::prelude::Vector::new(
                translation.x,
                translation.y,
                translation.z,
            ))
            .lock_rotations()
            .build();
        let rigid_body_handle = scene.physics_engine.add_rigid_body(rigid_body);
        scene
            .physics_engine
            .add_collider(collider, Some(rigid_body_handle));

        let rigidbody_component = RigidBody { rigid_body_handle };
        let controller = PlayerController::new(rigid_body_handle);

        entity.add_component(animation_component);
        entity.add_component(rigidbody_component);
        entity.add_component(ModelComponent::new(model));
        entity.add_component(controller);

        let bone_colliders = BoneColliders::build(scene, &entity);
        entity.add_component(bone_colliders);

        Ok(entity)
    }
}

impl PlayerController {
    pub fn new(rigid_body_handle: rapier3d::prelude::RigidBodyHandle) -> Self {
        Self {
            forward: 0.0,
            backward: 0.0,
            left: 0.0,
            right: 0.0,
            dirty: false,
            rigid_body_handle,
        }
    }
}

impl Component for PlayerController {
    fn update(&mut self, scene: &mut Scene, entity: &mut Entity, dt: f64) {
        if let Some(animation_component) = entity.get_component_mut::<AnimationComponent>() {
            if self.dirty {
                animation_component.set_input("forward", self.forward);
                animation_component.set_input("backward", self.backward);
                animation_component.set_input("left", self.left);
                animation_component.set_input("right", self.right);
            }
        }

        // Root motion from the animation drives horizontal movement.
        // reset_position() returns the accumulated displacement and clears it.
        let root_motion = if let Some(mc) = entity.get_component_mut::<ModelComponent>() {
            mc.get_model_mut().reset_position()
        } else {
            Vector3::zero()
        };

        // Preserve the vertical velocity already accumulated by physics (gravity).
        let current_vy = {
            let rb = &scene.physics_engine.rigid_bodies[self.rigid_body_handle];
            rb.linvel().y
        };

        // Convert per-frame displacement to velocity so the physics body tracks it.
        let hvel = if dt > 0.0 {
            root_motion / dt as f32
        } else {
            Vector3::zero()
        };

        {
            let rb = &mut scene.physics_engine.rigid_bodies[self.rigid_body_handle];
            rb.set_linvel(
                rapier3d::prelude::Vector::new(hvel.x, current_vy, hvel.z),
                true,
            );
        }

        // Keep the camera locked to the player (RigidBody::update already synced
        // entity position from physics this frame).
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
                Action::Press => { self.forward = 1.0; self.dirty = true; }
                Action::Release => { self.forward = 0.0; self.dirty = true; }
                _ => {}
            },
            glfw::WindowEvent::Key(Key::S, _, action, _) => match action {
                Action::Press => { self.backward = 1.0; self.dirty = true; }
                Action::Release => { self.backward = 0.0; self.dirty = true; }
                _ => {}
            },
            glfw::WindowEvent::Key(Key::A, _, action, _) => match action {
                Action::Press => { self.left = 1.0; self.dirty = true; }
                Action::Release => { self.left = 0.0; self.dirty = true; }
                _ => {}
            },
            glfw::WindowEvent::Key(Key::D, _, action, _) => match action {
                Action::Press => { self.right = 1.0; self.dirty = true; }
                Action::Release => { self.right = 0.0; self.dirty = true; }
                _ => {}
            },
            _ => {}
        }
    }
}
