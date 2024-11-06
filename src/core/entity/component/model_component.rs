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
    fn update(&mut self, _: &mut Scene, _: &mut Entity, delta_time: f64) {
        self.model.update(delta_time as f32);
    }

    fn render(&self, _scene: &Scene, parent_transform: &Matrix4<f32>) {
        if let Some(camera_component) = _scene.get_component::<CameraComponent>() {
            self.model.render(
                &parent_transform,
                &camera_component.get_camera(),
                &camera_component.get_projection(),
            );
        }
    }

    fn handle_event(&mut self, _: &mut glfw::Glfw, _: &mut glfw::Window, _: &glfw::WindowEvent) {}
}
