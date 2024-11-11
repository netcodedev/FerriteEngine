use cgmath::Matrix4;

use crate::core::{entity::Entity, model::Model, scene::Scene};

use super::{camera_component::CameraComponent, Component};

pub struct ModelComponent {
    model: Model,
}

impl ModelComponent {
    pub fn new(model: Model) -> Self {
        ModelComponent { model }
    }

    pub fn get_model(&self) -> &Model {
        &self.model
    }

    pub fn get_model_mut(&mut self) -> &mut Model {
        &mut self.model
    }
}

impl Component for ModelComponent {
    fn update(&mut self, _: &mut Scene, _: &mut Entity, _: f64) {}

    fn render(&self, _scene: &Scene, parent_transform: &Matrix4<f32>) {
        if let Some(camera_component) = _scene.get_component::<CameraComponent>() {
            let camera_projection = camera_component.get_projection().get_matrix() * camera_component.get_camera().get_matrix();
            self.model.render(
                &parent_transform,
                &camera_projection,
            );
        }
    }

    fn handle_event(&mut self, _: &mut glfw::Glfw, _: &mut glfw::Window, _: &glfw::WindowEvent) {}
}
