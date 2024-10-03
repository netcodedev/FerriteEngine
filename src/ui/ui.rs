use std::{cell::RefCell, rc::Rc};

use crate::plane::PlaneRenderer;

use super::{UIElement, UIRenderer};

impl UIRenderer {
    pub fn new(plane_renderer: Rc<RefCell<PlaneRenderer>>) -> Self {
        Self {
            plane_renderer,
            children: Vec::new(),
        }
    }

    pub fn add(&mut self, element: impl UIElement + 'static) {
        self.children.push(Box::new(element));
    }
    
    pub fn render(&mut self) {
        for child in &mut self.children {
            child.render(&self.plane_renderer.borrow());
        }
    }
    
    pub fn handle_events(&mut self, window: &mut glfw::Window, glfw: &mut glfw::Glfw, event: &glfw::WindowEvent) {
        for child in &mut self.children {
            child.handle_events(window, glfw, event);
        }
    }
}