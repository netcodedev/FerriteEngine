use crate::core::{
    renderer::{
        plane::{PlaneBuilder, PlaneRenderer},
        ui::UIElement,
    },
    scene::Scene,
};

use super::{Button, ButtonBuilder};

impl UIElement for Button {
    fn render(&mut self, scene: &mut Scene) {
        PlaneRenderer::render(&self.plane);
        for child in &mut self.children {
            child.render(scene);
        }
    }

    fn set_offset(&mut self, offset: (f32, f32)) {
        self.offset = offset;
        self.plane.set_position((
            self.offset.0 + self.position.0,
            self.offset.1 + self.position.1,
            0.0,
        ));
        for child in &mut self.children {
            child.set_offset((
                self.offset.0 + self.position.0,
                self.offset.1 + self.position.1,
            ));
        }
    }

    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut glfw::Window,
        _: &mut glfw::Glfw,
        event: &glfw::WindowEvent,
    ) -> bool {
        match event {
            glfw::WindowEvent::MouseButton(glfw::MouseButton::Button1, glfw::Action::Press, _) => {
                let (x, y) = window.get_cursor_pos();
                if x as f32 >= self.offset.0 + self.position.0
                    && x as f32 <= self.offset.0 + self.position.0 + self.size.0
                    && y as f32 >= self.offset.1 + self.position.1
                    && y as f32 <= self.offset.1 + self.position.1 + self.size.1
                {
                    (self.on_click)(scene);
                    return true;
                }
                false
            }
            glfw::WindowEvent::CursorPos(x, y) => {
                if *x as f32 >= self.offset.0 + self.position.0
                    && *x as f32 <= self.offset.0 + self.position.0 + self.size.0
                    && *y as f32 >= self.offset.1 + self.position.1
                    && *y as f32 <= self.offset.1 + self.position.1 + self.size.1
                {
                    if !self.is_hovering {
                        window.set_cursor(Some(glfw::Cursor::standard(glfw::StandardCursor::Hand)));
                        self.is_hovering = true;
                        self.plane.set_color((0.3, 0.4, 0.6, 1.0));
                    }
                } else if self.is_hovering {
                    window.set_cursor(None);
                    self.is_hovering = false;
                    self.plane.set_color((0.2, 0.3, 0.5, 1.0));
                }
                false
            }
            _ => false,
        }
    }

    fn add_children(&mut self, children: Vec<Box<dyn UIElement>>) {
        for mut child in children {
            child.set_offset((
                self.offset.0 + self.position.0,
                self.offset.1 + self.position.1,
            ));
            self.children.push(child);
        }
    }

    fn get_size(&self) -> (f32, f32) {
        self.size
    }
}

impl Button {
    pub fn new(position: (f32, f32), size: (f32, f32), on_click: Box<dyn Fn(&mut Scene)>) -> Self {
        Self {
            position,
            size,
            on_click,
            children: Vec::new(),
            offset: (0.0, 0.0),
            is_hovering: false,
            plane: PlaneBuilder::new()
                .position((position.0, position.1, 0.0))
                .size((size.0, size.1))
                .border_radius_uniform(5.0)
                .border_thickness(1.0)
                .color((0.2, 0.3, 0.5, 1.0))
                .build(),
        }
    }
}

impl ButtonBuilder {
    pub fn new() -> Self {
        Self {
            position: (0.0, 0.0),
            size: (0.0, 0.0),
            on_click: Box::new(|_| {}),
            children: Vec::new(),
        }
    }

    #[allow(dead_code)]
    pub fn position(mut self, x: f32, y: f32) -> Self {
        self.position = (x, y);
        self
    }

    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.size = (width, height);
        self
    }

    pub fn on_click(mut self, on_click: Box<dyn Fn(&mut Scene)>) -> Self {
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
