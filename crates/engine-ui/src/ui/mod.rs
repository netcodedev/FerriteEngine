use std::collections::BTreeMap;

use element_handle::UIElementHandle;
use ferrite::core::{
    primitives::{Offset, Size},
    scene::Scene,
};
use glfw::{Glfw, Window, WindowEvent};

pub mod clip;
pub mod theme;
pub mod element_handle;
pub mod elements;
mod ui;

pub trait UIElement {
    fn update(&mut self, scene: &mut Scene);
    fn render(&self);
    /// Second render pass for top-level overlays (modal dim layers, menus).
    /// Runs after every element's regular `render` so overlays sit above all
    /// sibling panels regardless of render order. Container-like elements
    /// should forward this call to their children; leaf elements default to
    /// a no-op.
    fn render_overlay(&self) {}
    /// While true, this top-level element gets every event before its
    /// siblings, so a modal is not shadowed by a panel underneath it.
    fn is_modal(&self) -> bool {
        false
    }
    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut Window,
        glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool;

    fn add_child(&mut self, child: Box<dyn UIElement>);
    fn add_child_to(&mut self, parent: UIElementHandle, child: Box<dyn UIElement>);
    fn contains_child(&self, handle: &UIElementHandle) -> bool;
    fn get_child(&self, handle: &UIElementHandle) -> Option<&Box<dyn UIElement>>;
    fn get_child_mut(&mut self, handle: &UIElementHandle) -> Option<&mut Box<dyn UIElement>>;

    fn get_handle(&self) -> &UIElementHandle;

    fn get_offset(&self) -> &Offset;
    fn set_offset(&mut self, offset: Offset);
    fn get_size(&self) -> &Size;
    fn set_z_index(&mut self, z_index: f32);
}

pub struct UI {
    children: BTreeMap<UIElementHandle, Box<dyn UIElement>>,
}
