use as_any::AsAny;

use glfw::{Glfw, Window};

use crate::core::scene::Scene;

pub trait Component: AsAny {
    fn update(&mut self, scene: &Scene, delta_time: f64);
    fn render(&self, _scene: &Scene) {}
    fn handle_event(&mut self, glfw: &mut Glfw, window: &mut Window, event: &glfw::WindowEvent);
}

pub mod camera_component;
