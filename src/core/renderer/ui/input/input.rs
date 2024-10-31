use crate::core::renderer::{
    plane::{PlaneBuilder, PlaneRenderer},
    text::TextRenderer,
    ui::UIElement,
};

use super::{Input, InputBuilder};

impl UIElement for Input {
    fn render(&mut self) {
        let mut plane = PlaneBuilder::new()
            .position((
                self.offset.0 + self.position.0,
                self.offset.1 + self.position.1,
                0.0,
            ))
            .size((self.size.0, self.size.1))
            .border_radius_uniform(5.0)
            .border_thickness(1.0);
        if self.is_hovering || self.is_focused {
            plane = plane.color((0.3, 0.3, 0.3, 1.0));
        } else {
            plane = plane.color((0.2, 0.2, 0.2, 1.0));
        }
        PlaneRenderer::render(plane.build());
        unsafe {
            gl::Enable(gl::DEPTH_TEST);
            gl::Enable(gl::STENCIL_TEST);
            gl::Clear(gl::STENCIL_BUFFER_BIT);
            gl::StencilFunc(gl::ALWAYS, 1, 0xFF);
            gl::StencilOp(gl::KEEP, gl::KEEP, gl::REPLACE);

            // Disable writing to the color and depth buffer
            gl::ColorMask(gl::FALSE, gl::FALSE, gl::FALSE, gl::FALSE);
            gl::DepthMask(gl::FALSE);

            // Render the plane to the stencil buffer
            let stencil_plane = plane.size((self.size.0 - 5.0, self.size.1)).build();
            PlaneRenderer::render(stencil_plane);
            gl::StencilFunc(gl::EQUAL, 1, 0xFF);
            gl::StencilMask(0x00);
            gl::Disable(gl::DEPTH_TEST);

            // Enable writing to the color and depth buffer
            gl::ColorMask(gl::TRUE, gl::TRUE, gl::TRUE, gl::TRUE);
            gl::DepthMask(gl::TRUE);

            if let Some(get_fn) = &self.get_fn {
                self.content = get_fn();
            }

            TextRenderer::render(
                (self.offset.0 + self.position.0 + 5.0) as i32,
                (self.offset.1 + self.position.1 + 5.0) as i32,
                16.0,
                &self.content,
            );
            gl::Disable(gl::STENCIL_TEST);
            gl::StencilMask(0xFF);
            gl::StencilFunc(gl::ALWAYS, 0, 0xFF);
        };
    }

    fn handle_events(
        &mut self,
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
                    self.is_focused = true;
                    return true;
                } else {
                    self.is_focused = false;
                }
                false
            }
            glfw::WindowEvent::CursorPos(x, y) => {
                if *x as f32 >= self.offset.0 + self.position.0
                    && *x as f32 <= self.offset.0 + self.position.0 + self.size.0
                    && *y as f32 >= self.offset.1 + self.position.1
                    && *y as f32 <= self.offset.1 + self.position.1 + self.size.1
                {
                    self.is_hovering = true;
                    window.set_cursor(Some(glfw::Cursor::standard(glfw::StandardCursor::IBeam)));
                } else {
                    if self.is_hovering {
                        window.set_cursor(None);
                        self.is_hovering = false;
                    }
                }
                false
            }
            glfw::WindowEvent::Char(character) => {
                if self.is_focused {
                    self.content.push(*character);
                    if let Some(set_fn) = &mut self.set_fn {
                        set_fn(self.content.clone());
                    }
                    return true;
                }
                false
            }
            glfw::WindowEvent::Key(
                glfw::Key::Backspace,
                _,
                glfw::Action::Press | glfw::Action::Repeat,
                _,
            ) => {
                if self.is_focused {
                    self.content.pop();
                    if let Some(set_fn) = &mut self.set_fn {
                        set_fn(self.content.clone());
                    }
                    return true;
                }
                false
            }
            glfw::WindowEvent::Key(_, _, glfw::Action::Press | glfw::Action::Repeat, _) => {
                if self.is_focused {
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    fn add_children(&mut self, _: Vec<Box<dyn UIElement>>) {}

    fn set_offset(&mut self, offset: (f32, f32)) {
        self.offset = offset;
    }

    fn get_size(&self) -> (f32, f32) {
        self.size
    }
}

impl Input {
    pub fn new(
        position: (f32, f32),
        size: (f32, f32),
        content: String,
        get_fn: Option<Box<dyn Fn() -> String>>,
        set_fn: Option<Box<dyn FnMut(String)>>,
    ) -> Self {
        Self {
            position,
            size,
            offset: (0.0, 0.0),
            is_hovering: false,
            is_focused: false,
            content,
            get_fn,
            set_fn,
        }
    }
}

impl InputBuilder {
    pub fn new(content: &str) -> Self {
        Self {
            position: (0.0, 0.0),
            size: (0.0, 0.0),
            content: content.to_string(),
            get_fn: None,
            set_fn: None,
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

    pub fn build(self) -> Input {
        Input::new(
            self.position,
            self.size,
            self.content,
            self.get_fn,
            self.set_fn,
        )
    }

    pub fn get_fn<F>(mut self, get_fn: F) -> Self
    where
        F: Fn() -> String + 'static,
    {
        self.get_fn = Some(Box::new(get_fn));
        self
    }

    pub fn set_fn<F>(mut self, set_fn: F) -> Self
    where
        F: FnMut(String) + 'static,
    {
        self.set_fn = Some(Box::new(set_fn));
        self
    }
}
