use glfw::{Glfw, WindowEvent};

use crate::core::entity::{component::Component, Entity};

use super::Scene;

impl Scene {
    pub fn new() -> Self {
        Scene {
            entities: Vec::new(),
        }
    }

    pub fn update(&mut self, delta_time: f64) {
        for entity in self.entities.iter_mut() {
            entity.update(delta_time);
        }
    }

    pub fn add_entity(&mut self, entity: Entity) {
        self.entities.push(entity);
    }

    pub fn handle_event(&mut self, glfw: &mut Glfw, window: &mut glfw::Window, event: &WindowEvent) {
        for entity in self.entities.iter_mut() {
            entity.handle_event(glfw, window, event);
        }
    }

    pub fn get_component<T>(&mut self) -> Option<&mut T> where T: Component {
        for entity in self.entities.iter_mut() {
            if let Some(component) = entity.get_component::<T>() {
                return Some(component);
            }
        }
        None
    }
}