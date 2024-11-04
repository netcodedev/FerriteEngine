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
        for i in 0..self.entities.len() {
            let mut entity = self.entities.remove(i);
            entity.update(self, delta_time);
            self.entities.insert(i, entity);
        }
    }

    pub fn render(&self) {
        for entity in self.entities.iter() {
            entity.render(self);
        }
    }

    pub fn add_entity(&mut self, entity: Entity) {
        self.entities.push(entity);
    }

    pub fn handle_event(
        &mut self,
        glfw: &mut Glfw,
        window: &mut glfw::Window,
        event: &WindowEvent,
    ) {
        for entity in self.entities.iter_mut() {
            entity.handle_event(glfw, window, event);
        }
    }

    pub fn get_component<T>(&self) -> Option<&T>
    where
        T: Component,
    {
        for entity in self.entities.iter() {
            if let Some(component) = entity.get_component::<T>() {
                return Some(component);
            }
        }
        None
    }

    pub fn get_component_mut<T>(&mut self) -> Option<&mut T>
    where
        T: Component,
    {
        for entity in self.entities.iter_mut() {
            if let Some(component) = entity.get_component_mut::<T>() {
                return Some(component);
            }
        }
        None
    }

    pub fn get_components<T>(&self) -> Vec<&T>
    where
        T: Component,
    {
        let mut components = Vec::new();
        for entity in self.entities.iter() {
            if let Some(component) = entity.get_component::<T>() {
                components.push(component);
            }
        }
        components
    }
}
