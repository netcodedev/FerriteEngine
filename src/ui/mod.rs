use std::{cell::RefCell, rc::Rc};

use crate::{plane::PlaneRenderer, text::TextRenderer};

pub mod button;
pub mod container;
pub mod input;
pub mod panel;
pub mod text;
pub mod ui;

pub struct UIRenderer {
    text_renderer: Rc<RefCell<TextRenderer>>,
    plane_renderer: Rc<RefCell<PlaneRenderer>>,
    children: Vec<Box<dyn UIElement>>,
}

pub trait UIElement {
    fn render(&self, text_renderer: &mut TextRenderer, plane_renderer: &PlaneRenderer);
    fn handle_events(&mut self, window: &mut glfw::Window, glfw: &mut glfw::Glfw, event: &glfw::WindowEvent) -> bool;
    fn add_children(&mut self, children: Vec<Box<dyn UIElement>>);
    fn set_offset(&mut self, offset: (f32, f32));
    fn get_size(&self) -> (f32, f32);
}