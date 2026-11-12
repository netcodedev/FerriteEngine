use cgmath::{EuclideanSpace, Matrix4, Point3, Quaternion};

use crate::core::{
    physics::{physics_engine::PhysicsEngine, rigidbody::RigidBody},
    scene::Scene,
    utils::DataSource,
};

use super::{component::Component, Entity, EntityHandle};

impl Entity {
    pub fn new(name: &str) -> Self {
        Entity {
            id: EntityHandle::new(),
            name: DataSource::new(name.to_string()),
            children: Vec::new(),
            components: Vec::new(),
            position: Point3::new(0.0, 0.0, 0.0),
            rotation: Quaternion::new(1.0, 0.0, 0.0, 0.0),
            transient: false,
        }
    }

    pub fn set_transient(&mut self, transient: bool) {
        self.transient = transient;
    }

    pub fn is_transient(&self) -> bool {
        self.transient
    }

    pub fn update_in_editor(&mut self, scene: &mut Scene, delta_time: f64) {
        for i in 0..self.components.len() {
            let mut component = self.components.remove(i);
            component.update_in_editor(scene, self, delta_time);
            self.components.insert(i, component);
        }
        for child in self.children.iter_mut() {
            child.update_in_editor(scene, delta_time);
        }
    }

    pub fn components(&self) -> &[Box<dyn Component>] {
        &self.components
    }

    pub fn components_mut(&mut self) -> &mut [Box<dyn Component>] {
        &mut self.components
    }

    pub fn remove_component_at(&mut self, index: usize) -> Option<Box<dyn Component>> {
        (index < self.components.len()).then(|| self.components.remove(index))
    }

    /// Like `get_component`, but never looks into children.
    pub fn get_own_component<T: Component>(&self) -> Option<&T> {
        self.components
            .iter()
            .find_map(|c| c.as_any().downcast_ref::<T>())
    }

    pub fn get_children_mut(&mut self) -> &mut Vec<Entity> {
        &mut self.children
    }

    /// Detach a descendant (at any depth) and return it.
    pub fn remove_descendant(&mut self, id: &EntityHandle) -> Option<Entity> {
        if let Some(index) = self.children.iter().position(|c| c.id == *id) {
            return Some(self.children.remove(index));
        }
        self.children
            .iter_mut()
            .find_map(|child| child.remove_descendant(id))
    }

    /// Call `on_remove` on every component in this subtree.
    pub fn release_resources(&mut self, physics: &mut PhysicsEngine) {
        for component in self.components.iter_mut() {
            component.on_remove(physics);
        }
        for child in self.children.iter_mut() {
            child.release_resources(physics);
        }
    }

    /// Set the local position without touching physics. Prefer
    /// `Scene::set_entity_position`, which also moves an attached body.
    pub fn set_local_position(&mut self, position: Point3<f32>) {
        self.position = position;
    }

    pub fn set_local_rotation(&mut self, rotation: Quaternion<f32>) {
        self.rotation = rotation;
    }

    /// Local transform (translation then rotation), as used when rendering.
    pub fn local_transform(&self) -> Matrix4<f32> {
        Matrix4::from_translation(self.position.to_vec()) * Matrix4::from(self.rotation)
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

    pub fn render_transparent(
        &self,
        scene: &Scene,
        view_projection: &Matrix4<f32>,
        parent_transform: Matrix4<f32>,
    ) {
        let transform = parent_transform
            * Matrix4::from_translation(self.position.to_vec())
            * Matrix4::from(self.rotation);
        for component in self.components.iter() {
            component.render_transparent(scene, self, view_projection, &transform);
        }

        for child in self.children.iter() {
            child.render_transparent(scene, view_projection, transform);
        }
    }

    pub fn add_child(&mut self, child: Entity) {
        self.children.push(child);
    }

    pub fn get_child(&self, id: &EntityHandle) -> Option<&Entity> {
        for child in self.children.iter() {
            if child.id == *id {
                return Some(child);
            }
            if let Some(entity) = child.get_child(id) {
                return Some(entity);
            }
        }
        None
    }

    pub fn get_children(&self) -> &Vec<Entity> {
        &self.children
    }

    pub fn get_child_mut(&mut self, id: &EntityHandle) -> Option<&mut Entity> {
        for child in self.children.iter_mut() {
            if child.id == *id {
                return Some(child);
            }
            if let Some(entity) = child.get_child_mut(id) {
                return Some(entity);
            }
        }
        None
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

    pub fn get_rotation(&self) -> Quaternion<f32> {
        self.rotation
    }

    pub fn set_position<P: Into<Point3<f32>>>(&mut self, scene: &mut Scene, position: P) {
        let position = position.into();
        self.position = position;
        if let Some(rigid_body) = self.get_component_mut::<RigidBody>() {
            rigid_body.set_position(scene, position);
        }
    }

    pub fn set_rotation(&mut self, scene: &mut Scene, rotation: Quaternion<f32>) {
        let _ = scene;
        self.rotation = rotation;
    }

    pub fn child_count(&self) -> usize {
        self.children.len()
    }

    pub fn get_name(&self) -> String {
        self.name.read()
    }

    pub fn set_name(&mut self, name: String) {
        self.name.write(name);
    }

    pub fn get_name_ref(&self) -> DataSource<String> {
        self.name.clone()
    }
}
