use cgmath::Matrix4;

use crate::core::{entity::Entity, model::Model, renderer::light::skylight, scene::Scene};

use super::Component;

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

    fn render(
        &self,
        scene: &Scene,
        view_projection: &Matrix4<f32>,
        parent_transform: &Matrix4<f32>,
    ) {
        if let Some(skylight) = scene.get_component::<skylight::SkyLight>() {
            self.model
                .render(&skylight.get_position(), &parent_transform, view_projection);
        }
    }

    fn handle_event(&mut self, _: &mut glfw::Glfw, _: &mut glfw::Window, _: &glfw::WindowEvent) {}
}
