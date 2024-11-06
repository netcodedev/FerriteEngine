use cgmath::Point3;

use crate::core::{entity::{component::{model_component::ModelComponent, Component}, Entity}, model::ModelBuilder};

use super::{Player, PlayerController};

impl Player {
    pub fn new<P: Into<Point3<f32>>>(position: P) -> Result<Entity, Box<dyn std::error::Error>> {
        let mut entity = Entity::new();
        entity.set_position(position);

        let mut model = ModelBuilder::new("Mannequin.fbx")?
            .with_animation("idle", "Idle.fbx")
            .with_animation("walk", "Walk.fbx")
            .with_animation("run", "Run.fbx")
            .build();
        model.init();
        model.blend_animations("walk", "run", 0.5, true);
        model.play_animation("idle");

        entity.add_component(ModelComponent::new(model));

        Ok(entity)
    }
}

impl Component for PlayerController {
    fn update(&mut self, scene: &crate::core::scene::Scene, delta_time: f64) {

    }

    fn handle_event(&mut self, glfw: &mut glfw::Glfw, window: &mut glfw::Window, event: &glfw::WindowEvent) {

    }
}