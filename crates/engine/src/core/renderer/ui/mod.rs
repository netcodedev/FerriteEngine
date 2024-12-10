use std::collections::BTreeMap;

use glfw::{Glfw, Window, WindowEvent};
use primitives::{Offset, Size, UIElementHandle};

use crate::core::scene::Scene;

pub mod button;
pub mod container;
pub mod input;
pub mod panel;
pub mod popup;
pub mod primitives;
pub mod text;
pub mod ui;

pub struct UI {}

pub struct UIRenderer {
    children: BTreeMap<UIElementHandle, Box<dyn UIElement>>,
}

pub trait UIElement {
    fn render(&mut self, scene: &mut Scene);
    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut Window,
        glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool;
    fn add_children(&mut self, children: Vec<(Option<UIElementHandle>, Box<dyn UIElement>)>);
    fn add_child_to(
        &mut self,
        parent: UIElementHandle,
        id: Option<UIElementHandle>,
        element: Box<dyn UIElement>,
    );
    fn contains_child(&self, handle: &UIElementHandle) -> bool;
    fn get_offset(&self) -> &Offset;
    fn set_offset(&mut self, offset: Offset);
    fn get_size(&self) -> &Size;
    fn set_z_index(&mut self, z_index: f32);
}
