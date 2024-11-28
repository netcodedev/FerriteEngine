use std::collections::HashMap;

use rand::Rng;

use crate::core::scene::Scene;

pub mod button;
pub mod container;
pub mod input;
pub mod panel;
pub mod text;
pub mod ui;

pub struct UI {}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct UIElementHandle(u64);

impl UIElementHandle {
    pub fn new() -> Self {
        Self {
            0: rand::thread_rng().gen::<u64>(),
        }
    }
    pub fn from(id: u64) -> Self {
        Self { 0: id }
    }
}

pub struct UIRenderer {
    children: HashMap<UIElementHandle, Box<dyn UIElement>>,
}

pub trait UIElement {
    fn render(&mut self, scene: &mut Scene);
    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut glfw::Window,
        glfw: &mut glfw::Glfw,
        event: &glfw::WindowEvent,
    ) -> bool;
    fn add_children(&mut self, children: Vec<(Option<UIElementHandle>, Box<dyn UIElement>)>);
    fn contains_child(&self, handle: &UIElementHandle) -> bool;
    fn get_offset(&self) -> (f32, f32);
    fn set_offset(&mut self, offset: (f32, f32));
    fn get_size(&self) -> (f32, f32);
}
