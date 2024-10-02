use std::{cell::RefCell, rc::Rc};

use crate::{plane::PlaneRenderer, text::TextRenderer};

pub mod ui;
pub mod button;
pub mod container;
pub mod text;

pub struct UIRenderer {
    text_renderer: Rc<RefCell<TextRenderer>>,
    plane_renderer: Rc<RefCell<PlaneRenderer>>,
    children: Vec<Box<dyn UIElement>>,
}

pub trait UIElement {
    fn render(&self, text_renderer: &mut TextRenderer, plane_renderer: &PlaneRenderer);
    fn handle_events(&self, window: &mut glfw::Window, event: &glfw::WindowEvent) -> bool;
    fn add_children(&mut self, children: Vec<Box<dyn UIElement>>);
}