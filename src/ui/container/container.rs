use crate::{plane::{PlaneBuilder, PlaneRenderer}, text::TextRenderer, ui::UIElement};

use super::{Container, ContainerBuilder};

impl Container {
    pub fn new(position: (f32, f32), size: (f32, f32)) -> Self {
        Self {
            position,
            size,
            children: Vec::new(),
            offset: (0.0, 0.0),
        }
    }
}

impl UIElement for Container {
    fn render(&self, text_renderer: &mut TextRenderer, plane_renderer: &PlaneRenderer) {
        plane_renderer.render(PlaneBuilder::new()
            .position((self.offset.0 + self.position.0, self.offset.1 + self.position.1, 0.0))
            .size((self.size.0, self.size.1))
            .color((0.2, 0.2, 0.2, 1.0))
            .border_radius_uniform(5.0)
            .border_thickness(1.0)
            .build(),
        1280,
        720
        );
        for child in &self.children {
            child.render(text_renderer, &plane_renderer);
        }
    }

    fn set_offset(&mut self, offset: (f32, f32)) {
        self.offset = offset;
        for child in &mut self.children {
            child.set_offset((self.offset.0 + self.position.0, self.offset.1 + self.position.1));
        }
    }

    fn handle_events(&self, window: &mut glfw::Window, event: &glfw::WindowEvent) -> bool {
        // test if click is within bounds
        match event {
            glfw::WindowEvent::MouseButton(glfw::MouseButton::Button1, glfw::Action::Press, _) => {
                let (x, y) = window.get_cursor_pos();
                if x as f32 >= self.offset.0 + self.position.0 &&
                    x as f32 <= self.offset.0 + self.position.0 + self.size.0 &&
                    y as f32 >= self.offset.1 + self.position.1 &&
                    y as f32 <= self.offset.1 + self.position.1 + self.size.1 {
                    for child in &self.children {
                        if child.handle_events(window, event) {
                            return true;
                        }
                    }
                }
                false
            }
            _ => false
        }
    }

    fn add_children(&mut self, children: Vec<Box<dyn UIElement>>) {
        for mut child in children {
            child.set_offset((self.offset.0 + self.position.0, self.offset.1 + self.position.1));
            self.children.push(child);
        }
    }
}

impl ContainerBuilder {
    pub fn new() -> Self {
        Self {
            position: (0.0, 0.0),
            size: (0.0, 0.0),
            children: Vec::new(),
        }
    }

    pub fn position(mut self, x: f32, y: f32) -> Self {
        self.position = (x, y);
        self
    }

    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.size = (width, height);
        self
    }

    pub fn add_child(mut self, child: Box<dyn UIElement>) -> Self {
        self.children.push(child);
        self
    }

    pub fn build(self) -> Container {
        let mut container = Container::new(self.position, self.size);
        container.add_children(self.children);
        container
    }
}