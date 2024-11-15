use cgmath::Point3;
use glfw::{Glfw, WindowEvent};
use rapier3d::prelude::*;

use crate::core::{
    entity::{component::Component, Entity},
    scene::Scene,
};

pub struct RigidBody {
    rigid_body_handle: RigidBodyHandle,
}

impl RigidBody {
    pub fn new(scene: &mut Scene, entity: &Entity) -> Self {
        let translation = entity.get_position();
        let rigid_body = RigidBodyBuilder::dynamic()
            .translation(vector![translation.x, translation.y, translation.z])
            .build();
        let rigid_body_handle = scene.physics_engine.add_rigid_body(rigid_body);
        scene
            .physics_engine
            .add_collider(ColliderBuilder::ball(1.0).build(), rigid_body_handle);
        RigidBody { rigid_body_handle }
    }

    pub fn set_position<P: Into<Point3<f32>>>(&mut self, scene: &mut Scene, position: P) {
        let position = position.into();
        let rigid_body = &mut scene.physics_engine.rigid_bodies[self.rigid_body_handle];
        rigid_body.set_position(
            Isometry::translation(position.x, position.y, position.z),
            true,
        );
    }
}

impl Component for RigidBody {
    fn update(&mut self, scene: &mut Scene, entity: &mut Entity, _: f64) {
        let translation = scene.physics_engine.rigid_bodies[self.rigid_body_handle].translation();
        entity.set_position(scene, (translation.x, translation.y, translation.z));
    }

    fn handle_event(&mut self, _: &mut Glfw, _: &mut glfw::Window, _: &WindowEvent) {}
}
