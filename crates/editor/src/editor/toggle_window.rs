use ferrite::core::{
    primitives::{Offset, Size},
    scene::Scene,
    utils::DataSource,
};
use ferrite_ui::ui::{element_handle::UIElementHandle, UIElement};
use glfw::{Glfw, WindowEvent};

/// Shows, updates and routes events to `inner` only while `visible` is true.
/// Used for the floating windows toggled from the Window menu.
pub struct ToggleWindow {
    handle: UIElementHandle,
    visible: DataSource<bool>,
    inner: Box<dyn UIElement>,
}

impl ToggleWindow {
    pub fn new(visible: DataSource<bool>, inner: Box<dyn UIElement>) -> Self {
        Self {
            handle: UIElementHandle::new(),
            visible,
            inner,
        }
    }
}

impl UIElement for ToggleWindow {
    fn update(&mut self, scene: &mut Scene) {
        if self.visible.read() {
            self.inner.update(scene);
        }
    }

    fn render(&self) {
        if self.visible.read() {
            self.inner.render();
        }
    }

    fn render_overlay(&self) {
        if self.visible.read() {
            self.inner.render_overlay();
        }
    }

    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut glfw::Window,
        glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        self.visible.read() && self.inner.handle_events(scene, window, glfw, event)
    }

    fn add_child(&mut self, child: Box<dyn UIElement>) {
        self.inner.add_child(child);
    }
    fn add_child_to(&mut self, parent: UIElementHandle, child: Box<dyn UIElement>) {
        self.inner.add_child_to(parent, child);
    }
    fn contains_child(&self, handle: &UIElementHandle) -> bool {
        self.inner.get_handle() == handle || self.inner.contains_child(handle)
    }
    fn get_child(&self, handle: &UIElementHandle) -> Option<&Box<dyn UIElement>> {
        if self.inner.get_handle() == handle {
            return Some(&self.inner);
        }
        self.inner.get_child(handle)
    }
    fn get_child_mut(&mut self, handle: &UIElementHandle) -> Option<&mut Box<dyn UIElement>> {
        if self.inner.get_handle() == handle {
            return Some(&mut self.inner);
        }
        self.inner.get_child_mut(handle)
    }
    fn get_handle(&self) -> &UIElementHandle {
        &self.handle
    }
    fn get_offset(&self) -> &Offset {
        self.inner.get_offset()
    }
    fn set_offset(&mut self, offset: Offset) {
        self.inner.set_offset(offset);
    }
    fn get_size(&self) -> &Size {
        self.inner.get_size()
    }
    fn set_z_index(&mut self, z_index: f32) {
        self.inner.set_z_index(z_index);
    }
}
