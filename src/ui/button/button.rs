use crate::{plane::{PlaneBuilder, PlaneRenderer}, text::TextRenderer, ui::UIElement};

use super::{Button, ButtonBuilder};

impl UIElement for Button {
    fn render(&self, text_renderer: &mut TextRenderer, plane_renderer: &PlaneRenderer) {
        plane_renderer.render(PlaneBuilder::new()
            .position((self.position.0, self.position.1, 0.0))
            .size((self.size.0, self.size.1))
            .color((0.3, 0.6, 0.7, 1.0))
            .build(),
        1280,
        720
        );
        for child in &self.children {
            child.render(text_renderer, &plane_renderer);
        }
    }

    fn handle_events(&self, window: &mut glfw::Window, event: &glfw::WindowEvent) -> bool {
        match event {
            glfw::WindowEvent::MouseButton(glfw::MouseButton::Button1, glfw::Action::Press, _) => {
                let (x, y) = window.get_cursor_pos();
                if x as f32 >= self.position.0 && x as f32 <= self.position.0 + self.size.0 && y as f32 >= self.position.1 && y as f32 <= self.position.1 + self.size.1 {
                    (self.on_click)();
                    return true
                }
                false
            }
            _ => false
        }
    }

    fn add_children(&mut self, children: Vec<Box<dyn UIElement>>) {
        self.children.extend(children);
    }
}

impl Button {
    pub fn new(position: (f32, f32), size: (f32, f32), on_click: Box<dyn Fn()>) -> Self {
        Self {
            position,
            size,
            on_click,
            children: Vec::new(),
        }
    }
}

impl ButtonBuilder {
    pub fn new() -> Self {
        Self {
            position: (0.0, 0.0),
            size: (0.0, 0.0),
            on_click: Box::new(|| {}),
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

    pub fn on_click(mut self, on_click: Box<dyn Fn()>) -> Self {
        self.on_click = on_click;
        self
    }

    pub fn add_child(mut self, child: Box<dyn UIElement>) -> Self {
        self.children.push(child);
        self
    }

    pub fn build(self) -> Button {
        let mut button = Button::new(self.position, self.size, self.on_click);
        button.add_children(self.children);
        button
    }
}