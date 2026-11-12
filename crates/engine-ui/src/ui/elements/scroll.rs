use std::collections::BTreeMap;

use ferrite::core::{
    primitives::{Offset, Position, Region, Size},
    renderer::plane::{Plane, PlaneBuilder, PlaneRenderer},
    scene::Scene,
};
use glfw::{
    Glfw, Window,
    WindowEvent::{self, Scroll},
};

use crate::ui::{element_handle::UIElementHandle, UIElement};

/// Vertically scrollable container. Children are laid out from top to bottom
/// with `gap` spacing. Scroll wheel updates the content offset.
pub struct ScrollArea {
    handle: UIElementHandle,
    region: Region,
    children: BTreeMap<UIElementHandle, Box<dyn UIElement>>,
    scroll_y: f32,
    content_height: f32,
    gap: f32,
    frame: Plane,
    stencil: Plane,
}

impl ScrollArea {
    pub fn new(position: Position, size: Size) -> Self {
        let frame = PlaneBuilder::new()
            .position(position)
            .size(size)
            .color((0.12, 0.12, 0.12, 1.0))
            .border_color((0.5, 0.5, 0.5, 1.0))
            .border_thickness(1.0)
            .border_radius_uniform(3.0)
            .build();
        let stencil = PlaneBuilder::new()
            .position(&position + (1.0, 1.0))
            .size(&size - (2.0, 2.0))
            .build();
        Self {
            handle: UIElementHandle::new(),
            region: Region::new(position, size),
            children: BTreeMap::new(),
            scroll_y: 0.0,
            content_height: 0.0,
            gap: 4.0,
            frame,
            stencil,
        }
    }

    fn layout(&mut self) {
        let base = &self.region.position + &self.region.offset;
        self.frame.set_position(base);
        self.stencil.set_position(&base + (1.0, 1.0));
        let mut y = self.gap - self.scroll_y;
        for child in self.children.values_mut() {
            child.set_offset(Offset {
                x: base.x + self.gap,
                y: base.y + y,
            });
            y += child.get_size().height + self.gap;
        }
        self.content_height = y + self.scroll_y;
    }
}

impl UIElement for ScrollArea {
    fn update(&mut self, scene: &mut Scene) {
        self.layout();
        for child in self.children.values_mut() {
            child.update(scene);
        }
    }

    fn render(&self) {
        self.frame.render();
        unsafe {
            gl::Enable(gl::STENCIL_TEST);
            gl::Clear(gl::STENCIL_BUFFER_BIT);
            gl::StencilFunc(gl::ALWAYS, 1, 0xFF);
            gl::StencilOp(gl::KEEP, gl::KEEP, gl::REPLACE);
            gl::ColorMask(gl::FALSE, gl::FALSE, gl::FALSE, gl::FALSE);
            gl::DepthMask(gl::FALSE);
            PlaneRenderer::render(&self.stencil);
            gl::StencilFunc(gl::EQUAL, 1, 0xFF);
            gl::StencilMask(0x00);
            gl::ColorMask(gl::TRUE, gl::TRUE, gl::TRUE, gl::TRUE);
            gl::DepthMask(gl::TRUE);
            gl::DepthFunc(gl::LEQUAL);
            for child in self.children.values() {
                child.render();
            }
            gl::Disable(gl::STENCIL_TEST);
            gl::StencilMask(0xFF);
            gl::StencilFunc(gl::ALWAYS, 0, 0xFF);
        }
    }

    fn render_overlay(&self) {
        for child in self.children.values() {
            child.render_overlay();
        }
    }

    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut Window,
        glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        if let Scroll(_, dy) = event {
            let (cx, cy) = window.get_cursor_pos();
            if self.region.contains(cx as f32, cy as f32) {
                let max_scroll = (self.content_height - self.region.size.height).max(0.0);
                self.scroll_y = (self.scroll_y - *dy as f32 * 20.0).clamp(0.0, max_scroll);
                return true;
            }
        }
        for child in self.children.values_mut() {
            if child.handle_events(scene, window, glfw, event) {
                return true;
            }
        }
        false
    }

    fn add_child(&mut self, mut child: Box<dyn UIElement>) {
        child.set_z_index(self.region.position.z + 1.0);
        self.children.insert(child.get_handle().clone(), child);
    }
    fn add_child_to(&mut self, parent: UIElementHandle, child: Box<dyn UIElement>) {
        if let Some(p) = self.children.get_mut(&parent) {
            p.add_child(child);
        } else {
            for c in self.children.values_mut() {
                if c.contains_child(&parent) {
                    c.add_child_to(parent, child);
                    return;
                }
            }
        }
    }
    fn contains_child(&self, handle: &UIElementHandle) -> bool {
        self.children.contains_key(handle)
            || self.children.values().any(|c| c.contains_child(handle))
    }
    fn get_child(&self, handle: &UIElementHandle) -> Option<&Box<dyn UIElement>> {
        if let Some(c) = self.children.get(handle) {
            return Some(c);
        }
        for c in self.children.values() {
            if let Some(x) = c.get_child(handle) {
                return Some(x);
            }
        }
        None
    }
    fn get_child_mut(&mut self, handle: &UIElementHandle) -> Option<&mut Box<dyn UIElement>> {
        for (h, c) in self.children.iter_mut() {
            if h == handle {
                return Some(c);
            }
            if let Some(x) = c.get_child_mut(handle) {
                return Some(x);
            }
        }
        None
    }

    fn get_handle(&self) -> &UIElementHandle {
        &self.handle
    }
    fn get_offset(&self) -> &Offset {
        &self.region.offset
    }
    fn set_offset(&mut self, offset: Offset) {
        self.region.offset = offset;
    }
    fn get_size(&self) -> &Size {
        &self.region.size
    }
    fn set_z_index(&mut self, z_index: f32) {
        self.region.position.z = z_index;
        self.frame.set_z_index(z_index);
        self.stencil.set_z_index(z_index + 1.0);
        for c in self.children.values_mut() {
            c.set_z_index(z_index + 2.0);
        }
    }
}
