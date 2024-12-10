use core::panic;
use std::str::FromStr;

use log::debug;

use crate::core::{
    renderer::{
        plane::{PlaneBuilder, PlaneRenderer},
        text::{Fonts, Text},
        ui::{
            primitives::{Position, Region},
            Offset, Size, UIElement, UIElementHandle,
        },
    },
    scene::Scene,
    utils::DataSource,
};

use super::{Input, InputBuilder};

impl<T: Clone + ToString + FromStr> UIElement for Input<T> {
    fn render(&mut self, _: &mut Scene) {
        PlaneRenderer::render(&self.plane);
        unsafe {
            gl::Enable(gl::DEPTH_TEST);
            gl::Disable(gl::STENCIL_TEST);
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

            if let Some(data_source) = &self.data_source {
                self.content = data_source.to_string();
            }
            self.text.set_content(&self.content);
            self.text
                .render_at(&(&self.position + &self.offset) + (5.0, 2.0, 1.0));
            gl::Disable(gl::STENCIL_TEST);
            gl::StencilMask(0xFF);
            gl::StencilFunc(gl::ALWAYS, 0, 0xFF);
        };
    }

    fn handle_events(
        &mut self,
        _: &mut Scene,
        window: &mut glfw::Window,
        _: &mut glfw::Glfw,
        event: &glfw::WindowEvent,
    ) -> bool {
        let region = Region::new_with_offset(self.position, self.size, self.offset);
        match event {
            glfw::WindowEvent::MouseButton(glfw::MouseButton::Button1, glfw::Action::Press, _) => {
                let (x, y) = window.get_cursor_pos();
                let (x, y) = (x as f32, y as f32);
                if region.contains(x, y) {
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
                if region.contains(*x as f32, *y as f32) {
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
                    if let Some(data_source) = &self.data_source {
                        data_source.write_from_string(self.content.clone());
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
                    if let Some(data_source) = &mut self.data_source {
                        data_source.write_from_string(self.content.clone());
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
        self.plane.set_position(&self.position + &self.offset);
        self.stencil_plane
            .set_position(&self.position + &self.offset);
    }

    fn get_size(&self) -> &Size {
        &self.size
    }

    fn contains_child(&self, _: &UIElementHandle) -> bool {
        false
    }

    fn get_offset(&self) -> &Offset {
        &self.offset
    }

    fn add_child_to(
        &mut self,
        _: UIElementHandle,
        _: Option<UIElementHandle>,
        _: Box<dyn UIElement>,
    ) {
        panic!("Input cannot have children");
    }

    fn set_z_index(&mut self, z_index: f32) {
        debug!("Setting z index of input to {}", z_index);
        self.position.z = z_index;
        self.plane.set_z_index(z_index);
        self.stencil_plane.set_z_index(z_index + 1.0);
        self.text.set_z_index(z_index + 1.0);
    }
}

impl<T: Clone + ToString> Input<T> {
    pub fn new(
        position: Position,
        size: Size,
        content: T,
        data_source: Option<DataSource<T>>,
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
            content: content.to_string(),
            text: Text::new(Fonts::RobotoMono, 0, 0, 0, 16.0, content.to_string()),
            plane: plane.build(),
            stencil_plane: plane
                .size(Size {
                    width: size.width - 12.0,
                    height: size.height,
                })
                .build(),
            data_source,
        }
    }
}

impl<T: Clone + ToString> InputBuilder<T> {
    pub fn new(content: T) -> Self {
        Self {
            position: Position::default(),
            size: Size::default(),
            content,
            data_source: None,
        }
    }

    pub fn position(mut self, x: f32, y: f32) -> Self {
        self.position = Position { x, y, z: 0.0 };
        self
    }

    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.size = Size { width, height };
        self
    }

    pub fn data_source(mut self, data_source: Option<DataSource<T>>) -> Self {
        self.data_source = data_source;
        self
    }

    pub fn build(self) -> Input<T> {
        Input::new(self.position, self.size, self.content, self.data_source)
    }
}
