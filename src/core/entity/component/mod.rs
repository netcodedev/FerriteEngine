use as_any::AsAny;

use glfw::{Glfw, Window};

pub trait Component: AsAny {
    fn update(&mut self, delta_time: f64);
    fn handle_event(&mut self, glfw: &mut Glfw, window: &mut Window, event: &glfw::WindowEvent);
}

pub mod camera_component;