use cgmath::Point3;
use glfw::{Glfw, WindowEvent};
use rapier3d::prelude::*;

use crate::core::{
    entity::{component::Component, Entity},
    scene::Scene,
};

pub struct RigidBody {
    pub rigid_body_handle: RigidBodyHandle,
}

impl RigidBody {
    pub fn new(
        rigid_body_type: RigidBodyType,
        scene: &mut Scene,
        entity: &Entity,
        collider: Option<Collider>,
    ) -> Self {
        let translation = entity.get_position();
        let rigid_body_builder = match rigid_body_type {
            RigidBodyType::Fixed => RigidBodyBuilder::fixed(),
            RigidBodyType::Dynamic => RigidBodyBuilder::dynamic(),
            RigidBodyType::KinematicPositionBased => RigidBodyBuilder::kinematic_position_based(),
            RigidBodyType::KinematicVelocityBased => RigidBodyBuilder::kinematic_velocity_based(),
        };
        let rigid_body = rigid_body_builder
            .translation(Vector::new(translation.x, translation.y, translation.z))
            .build();
        let rigid_body_handle = scene.physics_engine.add_rigid_body(rigid_body);
        if let Some(collider) = collider {
            scene
                .physics_engine
                .add_collider(collider, Some(rigid_body_handle));
        }
        RigidBody { rigid_body_handle }
    }

    pub fn set_position<P: Into<Point3<f32>>>(&mut self, scene: &mut Scene, position: P) {
        let position = position.into();
        let rigid_body = &mut scene.physics_engine.rigid_bodies[self.rigid_body_handle];
        rigid_body.set_translation(Vector::new(position.x, position.y, position.z), true);
    }

    pub fn set_linvel(&mut self, scene: &mut Scene, linvel: cgmath::Vector3<f32>) {
        let rigid_body = &mut scene.physics_engine.rigid_bodies[self.rigid_body_handle];
        rigid_body.set_linvel(Vector::new(linvel.x, linvel.y, linvel.z), true);
    }

    pub fn get_linvel(&self, scene: &Scene) -> cgmath::Vector3<f32> {
        let rigid_body = &scene.physics_engine.rigid_bodies[self.rigid_body_handle];
        let v = rigid_body.linvel();
        cgmath::Vector3::new(v.x, v.y, v.z)
    }
}

impl Component for RigidBody {
    fn update(&mut self, scene: &mut Scene, entity: &mut Entity, _: f64) {
        let rigidbody = &scene.physics_engine.rigid_bodies[self.rigid_body_handle];
        let translation = rigidbody.translation();
        let rotation = rigidbody.rotation();
        let quat = cgmath::Quaternion::new(rotation.w, rotation.x, rotation.y, rotation.z);
        entity.set_position(scene, (translation.x, translation.y, translation.z));
        entity.set_rotation(scene, quat);
    }

    fn handle_event(&mut self, _: &mut Glfw, _: &mut glfw::Window, _: &WindowEvent) {}
}
