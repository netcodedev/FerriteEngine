use cgmath::{Matrix4, Point3, Vector3};
use glfw::{Glfw, WindowEvent};

use crate::core::{
    entity::{component::Component, Entity},
    scene::Scene,
};

pub struct SkyLight {
    position: Point3<f32>,
}

impl SkyLight {
    pub fn new<P: Into<Point3<f32>>>(position: P) -> Self {
        Self {
            position: position.into(),
        }
    }

    pub fn get_position(&self) -> Point3<f32> {
        self.position
    }

    pub fn get_projection(&self) -> Matrix4<f32> {
        let light_projection = cgmath::ortho(-10.0, 10.0, -10.0, 10.0, 0.1, 20.0);
        let light_view = Matrix4::look_at_rh(
            self.position,
            Point3::new(0.0, 50.0, 0.0),
            Vector3::new(0.0, 1.0, 0.0),
        );

        light_projection * light_view
    }
}

impl Component for SkyLight {
    fn update(&mut self, _: &mut Scene, _: &mut Entity, _: f64) {}

    fn handle_event(&mut self, _: &mut Glfw, _: &mut glfw::Window, _: &WindowEvent) {}
}
