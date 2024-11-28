use cgmath::{EuclideanSpace, Matrix4, Point3, Quaternion};
use rand::Rng;

use crate::core::{physics::rigidbody::RigidBody, scene::Scene};

use super::{component::Component, Entity};

#[allow(dead_code)]
impl Entity {
    pub fn new(name: &str) -> Self {
        Entity {
            id: rand::thread_rng().gen(),
            name: name.to_string(),
            children: Vec::new(),
            components: Vec::new(),
            position: Point3::new(0.0, 0.0, 0.0),
            rotation: Quaternion::new(1.0, 0.0, 0.0, 0.0),
        }
    }

    pub fn update(&mut self, scene: &mut Scene, delta_time: f64) {
        for i in 0..self.components.len() {
            let mut component = self.components.remove(i);
            component.update(scene, self, delta_time);
            self.components.insert(i, component);
        }

        for child in self.children.iter_mut() {
            child.update(scene, delta_time);
        }
    }

    pub fn render(
        &self,
        scene: &Scene,
        view_projection: &Matrix4<f32>,
        parent_transform: Matrix4<f32>,
    ) {
        let transform = parent_transform
            * Matrix4::from_translation(self.position.to_vec())
            * Matrix4::from(self.rotation);
        for component in self.components.iter() {
            component.render(scene, self, view_projection, &transform);
        }

        for child in self.children.iter() {
            child.render(scene, view_projection, transform);
        }
    }

    pub fn add_child(&mut self, child: Entity) {
        self.children.push(child);
    }

    pub fn handle_event(
        &mut self,
        glfw: &mut glfw::Glfw,
        window: &mut glfw::Window,
        event: &glfw::WindowEvent,
    ) {
        for component in self.components.iter_mut() {
            component.handle_event(glfw, window, event);
        }

        for child in self.children.iter_mut() {
            child.handle_event(glfw, window, event);
        }
    }

    pub fn add_component<T: 'static + Component>(&mut self, component: T) {
        self.components.push(Box::new(component));
    }

    pub fn get_component<T>(&self) -> Option<&T>
    where
        T: Component,
    {
        for component in self.components.iter() {
            if let Some(component) = component.as_any().downcast_ref::<T>() {
                return Some(component);
            }
        }
        for child in self.children.iter() {
            if let Some(component) = child.get_component::<T>() {
                return Some(component);
            }
        }
        None
    }

    pub fn get_with_own_component<T>(&self) -> Vec<&Entity>
    where
        T: Component,
    {
        let mut entities = Vec::new();
        for component in self.components.iter() {
            if let Some(_) = component.as_any().downcast_ref::<T>() {
                entities.push(self);
            }
        }
        for child in self.children.iter() {
            entities.append(&mut child.get_with_own_component::<T>());
        }
        entities
    }

    pub fn get_component_mut<T>(&mut self) -> Option<&mut T>
    where
        T: Component,
    {
        for component in self.components.iter_mut() {
            if let Some(component) = component.as_any_mut().downcast_mut::<T>() {
                return Some(component);
            }
        }
        None
    }

    pub fn get_position(&self) -> Point3<f32> {
        self.position
    }

    pub fn set_position<P: Into<Point3<f32>>>(&mut self, scene: &mut Scene, position: P) {
        let position = position.into();
        self.position = position;
        if let Some(rigid_body) = self.get_component_mut::<RigidBody>() {
            rigid_body.set_position(scene, position);
        }
    }

    pub fn set_rotation(&mut self, scene: &mut Scene, rotation: Quaternion<f32>) {
        self.rotation = rotation;
        if let Some(rigid_body) = self.get_component_mut::<RigidBody>() {
            rigid_body.set_rotation(scene, rotation);
        }
    }

    pub fn child_count(&self) -> usize {
        self.children.len()
    }

    pub fn get_name(&self) -> String {
        self.name.clone()
    }
}
