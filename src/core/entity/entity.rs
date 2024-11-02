use super::{component::Component, Entity};

#[allow(dead_code)]
impl Entity {
    pub fn new() -> Self {
        Entity {
            children: Vec::new(),
            components: Vec::new(),
        }
    }

    pub fn update(&mut self, delta_time: f64) {
        for component in self.components.iter_mut() {
            component.update(delta_time);
        }

        for child in self.children.iter_mut() {
            child.update(delta_time);
        }
    }

    pub fn add_child(&mut self, child: Entity) {
        self.children.push(child);
    }

    pub fn handle_event(&mut self, glfw: &mut glfw::Glfw, window: &mut glfw::Window, event: &glfw::WindowEvent) {
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

    pub fn get_component<T>(&mut self) -> Option<&mut T> where T: Component {
        for component in self.components.iter_mut() {
            if let Some(component) = component.as_any_mut().downcast_mut::<T>() {
                return Some(component);
            }
        }
        None
    }
}