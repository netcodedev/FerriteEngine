use std::{cell::RefCell, rc::Rc};

use crate::{plane::PlaneRenderer, text::TextRenderer};

use super::{UIElement, UIRenderer};

impl UIRenderer {
    pub fn new(text_renderer: Rc<RefCell<TextRenderer>>, plane_renderer: Rc<RefCell<PlaneRenderer>>) -> Self {
        Self {
            text_renderer,
            plane_renderer,
            children: Vec::new(),
        }
    }

    pub fn add(&mut self, element: impl UIElement + 'static) {
        self.children.push(Box::new(element));
    }
    
    pub fn render(&self) {
        for child in &self.children {
            child.render(&mut self.text_renderer.borrow_mut(), &self.plane_renderer.borrow());
        }
    }
    
    pub fn handle_events(&mut self, window: &mut glfw::Window, event: &glfw::WindowEvent) {
        for child in &mut self.children {
            child.handle_events(window, event);
        }
    }
}