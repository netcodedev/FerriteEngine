use core::panic;

use crate::core::{
    entity::EntityHandle,
    renderer::{
        plane::{PlaneBuilder, PlaneRenderer},
        text::{Fonts, Text},
        ui::{
            primitives::{Offset, Position, Size},
            UIElement, UIElementHandle,
        },
    },
    scene::Scene,
};

use super::{Input, InputBuilder};

impl UIElement for Input {
    fn render(&mut self, scene: &mut Scene) {
        PlaneRenderer::render(&self.plane);
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
            PlaneRenderer::render(&self.stencil_plane);
            gl::StencilFunc(gl::EQUAL, 1, 0xFF);
            gl::StencilMask(0x00);
            gl::Disable(gl::DEPTH_TEST);

            // Enable writing to the color and depth buffer
            gl::ColorMask(gl::TRUE, gl::TRUE, gl::TRUE, gl::TRUE);
            gl::DepthMask(gl::TRUE);

            if let Some(get_fn) = &self.get_fn {
                self.content = get_fn(&self.entity_handle, scene);
            }
            self.text.set_content(self.content.clone());
            self.text.render_at(
                (self.offset.x + self.position.x + 5.0) as i32,
                (self.offset.y + self.position.y + 2.0) as i32,
            );
            gl::Disable(gl::STENCIL_TEST);
            gl::StencilMask(0xFF);
            gl::StencilFunc(gl::ALWAYS, 0, 0xFF);
        };
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
                if x as f32 >= self.offset.x + self.position.x
                    && x as f32 <= self.offset.x + self.position.x + self.size.width
                    && y as f32 >= self.offset.y + self.position.y
                    && y as f32 <= self.offset.y + self.position.y + self.size.height
                {
                    if !self.is_focused {
                        self.is_focused = true;
                        self.plane.set_color((0.3, 0.3, 0.3, 1.0));
                        self.stencil_plane.set_color((0.3, 0.3, 0.3, 1.0));
                    }
                    return true;
                } else if self.is_focused {
                    self.is_focused = false;
                    self.plane.set_color((0.2, 0.2, 0.2, 1.0));
                    self.stencil_plane.set_color((0.2, 0.2, 0.2, 1.0));
                }
                false
            }
            glfw::WindowEvent::CursorPos(x, y) => {
                if *x as f32 >= self.offset.x + self.position.x
                    && *x as f32 <= self.offset.x + self.position.x + self.size.width
                    && *y as f32 >= self.offset.y + self.position.y
                    && *y as f32 <= self.offset.y + self.position.y + self.size.height
                {
                    if !self.is_hovering {
                        self.is_hovering = true;
                        self.plane.set_color((0.3, 0.3, 0.3, 1.0));
                        self.stencil_plane.set_color((0.3, 0.3, 0.3, 1.0));
                        window
                            .set_cursor(Some(glfw::Cursor::standard(glfw::StandardCursor::IBeam)));
                    }
                } else if self.is_hovering {
                    window.set_cursor(None);
                    self.is_hovering = false;
                    if !self.is_focused {
                        self.plane.set_color((0.2, 0.2, 0.2, 1.0));
                        self.stencil_plane.set_color((0.2, 0.2, 0.2, 1.0));
                    }
                }
                false
            }
            glfw::WindowEvent::Char(character) => {
                if self.is_focused {
                    self.content.push(*character);
                    if let Some(set_fn) = &mut self.set_fn {
                        set_fn(&self.entity_handle, scene, self.content.clone());
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
                        set_fn(&self.entity_handle, scene, self.content.clone());
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

    fn add_children(&mut self, _: Vec<(Option<UIElementHandle>, Box<dyn UIElement>)>) {
        panic!("Input cannot have children");
    }

    fn set_offset(&mut self, offset: Offset) {
        self.offset = offset;
        self.plane.set_position(self.position + &self.offset);
        self.stencil_plane
            .set_position(self.position + &self.offset);
    }

    fn get_size(&self) -> Size {
        self.size
    }

    fn contains_child(&self, _: &UIElementHandle) -> bool {
        false
    }

    fn get_offset(&self) -> Offset {
        self.offset
    }

    fn add_child_to(
        &mut self,
        _: UIElementHandle,
        _: Option<UIElementHandle>,
        _: Box<dyn UIElement>,
    ) {
        panic!("Input cannot have children");
    }
}

impl Input {
    pub fn new(
        position: Position,
        size: Size,
        content: String,
        get_fn: Option<Box<dyn Fn(&Option<EntityHandle>, &mut Scene) -> String>>,
        set_fn: Option<Box<dyn FnMut(&Option<EntityHandle>, &mut Scene, String)>>,
        entity_handle: Option<EntityHandle>,
    ) -> Self {
        let plane = PlaneBuilder::new()
            .position(position)
            .size(Size {
                width: size.width - 10.0,
                height: size.height,
            })
            .border_radius_uniform(5.0)
            .border_thickness(1.0);
        Self {
            position,
            size,
            offset: Offset::default(),
            is_hovering: false,
            is_focused: false,
            content: content.clone(),
            get_fn,
            set_fn,
            text: Text::new(Fonts::RobotoMono, 0, 0, 16.0, content),
            plane: plane.build(),
            stencil_plane: plane
                .size(Size {
                    width: size.width - 12.0,
                    height: size.height,
                })
                .build(),
            entity_handle,
        }
    }
}

impl InputBuilder {
    pub fn new(content: &str) -> Self {
        Self {
            position: Position::default(),
            size: Size::default(),
            content: content.to_string(),
            get_fn: None,
            set_fn: None,
            entity_handle: None,
        }
    }

    pub fn position(mut self, x: f32, y: f32) -> Self {
        self.position = Position { x, y };
        self
    }

    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.size = Size { width, height };
        self
    }

    pub fn entity_handle(mut self, entity_handle: EntityHandle) -> Self {
        self.entity_handle = Some(entity_handle);
        self
    }

    pub fn build(self) -> Input {
        Input::new(
            self.position,
            self.size,
            self.content,
            self.get_fn,
            self.set_fn,
            self.entity_handle,
        )
    }

    pub fn get_fn<F>(mut self, get_fn: F) -> Self
    where
        F: Fn(&Option<EntityHandle>, &mut Scene) -> String + 'static,
    {
        self.get_fn = Some(Box::new(get_fn));
        self
    }

    pub fn set_fn<F>(mut self, set_fn: F) -> Self
    where
        F: FnMut(&Option<EntityHandle>, &mut Scene, String) + 'static,
    {
        self.set_fn = Some(Box::new(set_fn));
        self
    }
}
