use core::panic;
use std::str::FromStr;

use ferrite::core::{
    primitives::{Offset, Position, Region, Size},
    renderer::{
        plane::{PlaneBuilder, PlaneRenderer},
        text::{Fonts, Text},
    },
    scene::Scene,
    utils::DataSource,
};
use glfw::Action::{Press, Repeat};
use glfw::Key::Backspace;
use glfw::WindowEvent::{Char, CursorPos, Key, MouseButton};

use crate::ui::{element_handle::UIElementHandle, UIElement};

use super::Input;

impl<T: Clone + ToString> Input<T> {
    pub fn new(
        position: Position,
        size: Size,
        content: T,
        data_source: Option<DataSource<T>>,
    ) -> Self {
        let plane = PlaneBuilder::new()
            .position(position)
            .size(size)
            .border_radius_uniform(5.0)
            .border_thickness(1.0);
        Self {
            handle: UIElementHandle::new(),
            region: Region::new(position, size),
            is_hovering: false,
            is_focused: false,
            content: content.to_string(),
            text: Text::new(Fonts::RobotoMono, 0, 0, 0, 16.0, content.to_string()),
            plane: plane.build(),
            stencil_plane: plane.size(&size - (2.0, 0.0)).build(),
            data_source,
        }
    }

    pub fn set_handle(&mut self, handle: UIElementHandle) {
        self.handle = handle;
    }

    pub fn set_size(&mut self, size: Size) {
        self.region.size = size;
        self.plane.set_size(Size {
            width: size.width,
            height: size.height,
        });
        self.stencil_plane.set_size(Size {
            width: size.width - 2.0,
            height: size.height,
        });
    }
}

impl<T: Clone + ToString + FromStr> UIElement for Input<T> {
    fn update(&mut self, _scene: &mut Scene) {
        if let Some(data_source) = &self.data_source {
            self.content = data_source.to_string();
        }
        self.text.set_content(&self.content);
        self.text
            .prepare_render_at(&(&self.region.position + &self.region.offset) + (5.0, 2.0, 1.0));
    }

    fn render(&self) {
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

            // Enable writing to the color and depth buffer
            gl::ColorMask(gl::TRUE, gl::TRUE, gl::TRUE, gl::TRUE);
            gl::DepthMask(gl::TRUE);
            gl::DepthFunc(gl::LEQUAL);

            self.text.render();
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
        match event {
            MouseButton(glfw::MouseButton::Button1, glfw::Action::Press, _) => {
                let (x, y) = window.get_cursor_pos();
                let (x, y) = (x as f32, y as f32);
                if self.region.contains(x, y) {
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
            CursorPos(x, y) => {
                if self.region.contains(*x as f32, *y as f32) {
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
            Char(character) => {
                if self.is_focused {
                    self.content.push(*character);
                    if let Some(data_source) = &self.data_source {
                        data_source.write_from_string(self.content.clone());
                    }
                    return true;
                }
                false
            }
            Key(Backspace, _, Press | Repeat, _) => {
                if self.is_focused {
                    self.content.pop();
                    if let Some(data_source) = &mut self.data_source {
                        data_source.write_from_string(self.content.clone());
                    }
                    return true;
                }
                false
            }
            Key(_, _, Press | Repeat, _) => {
                if self.is_focused {
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    fn add_child(&mut self, _: Box<dyn UIElement>) {
        panic!("Input cannot have children");
    }

    fn get_handle(&self) -> &UIElementHandle {
        &self.handle
    }

    fn set_offset(&mut self, offset: Offset) {
        self.region.offset = offset;
        self.plane
            .set_position(&self.region.position + &self.region.offset);
        self.stencil_plane
            .set_position(&self.region.position + &self.region.offset);
    }

    fn get_size(&self) -> &Size {
        &self.region.size
    }

    fn contains_child(&self, _: &UIElementHandle) -> bool {
        false
    }

    fn get_child(&self, _handle: &UIElementHandle) -> Option<&Box<dyn UIElement>> {
        None
    }

    fn get_child_mut(&mut self, _handle: &UIElementHandle) -> Option<&mut Box<dyn UIElement>> {
        None
    }

    fn get_offset(&self) -> &Offset {
        &self.region.offset
    }

    fn add_child_to(&mut self, _: UIElementHandle, _: Box<dyn UIElement>) {
        panic!("Input cannot have children");
    }

    fn set_z_index(&mut self, z_index: f32) {
        self.region.position.z = z_index;
        self.plane.set_z_index(z_index);
        self.stencil_plane.set_z_index(z_index + 1.0);
        self.text.set_z_index(z_index + 1.0);
    }
}
