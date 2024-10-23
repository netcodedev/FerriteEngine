use super::{UIElement, UIRenderer};

impl UIRenderer {
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
        }
    }

    pub fn add(&mut self, element: impl UIElement + 'static) {
        self.children.push(Box::new(element));
    }
    
    pub fn render(&mut self) {
        for child in &mut self.children {
            child.render();
        }
    }
    
    pub fn handle_events(&mut self, window: &mut glfw::Window, glfw: &mut glfw::Glfw, event: &glfw::WindowEvent) -> bool {
        for child in &mut self.children {
            if child.handle_events(window, glfw, event) {
                return true;
            }
        }
        false
    }
}