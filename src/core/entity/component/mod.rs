use as_any::AsAny;

use cgmath::Matrix4;
use glfw::{Glfw, Window};

use crate::core::scene::Scene;

use super::Entity;

pub trait Component: AsAny {
    fn update(&mut self, scene: &mut Scene, entity: &mut Entity, delta_time: f64);
    fn render(
        &self,
        _scene: &Scene,
        _entity: &Entity,
        _view_projection: &Matrix4<f32>,
        _parent_transform: &Matrix4<f32>,
    ) {
    }
    fn handle_event(&mut self, glfw: &mut Glfw, window: &mut Window, event: &glfw::WindowEvent);
}

pub mod animation_component;
pub mod camera_component;
pub mod debug_component;
pub mod model_component;
