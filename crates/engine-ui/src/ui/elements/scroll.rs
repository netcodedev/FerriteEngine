use crate::ui::theme;
use std::collections::BTreeMap;

use ferrite::core::{
    primitives::{Offset, Position, Region, Size},
    renderer::plane::{Plane, PlaneBuilder},
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
    thumb: Plane,
}

impl ScrollArea {
    pub fn new(position: Position, size: Size) -> Self {
        let frame = PlaneBuilder::new()
            .position(position)
            .size(size)
            .color(theme::TRANSPARENT)
            .border_color(theme::TRANSPARENT)
            .border_thickness(1.0)
            .border_radius_uniform(theme::RADIUS)
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
            thumb: PlaneBuilder::new()
                .color(theme::BORDER_STRONG)
                .border_radius_uniform(2.0)
                .build(),
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

        let view = self.region.size.height;
        if self.content_height > view + 1.0 {
            let ratio = view / self.content_height;
            let thumb_h = (view * ratio).max(24.0);
            let max_scroll = self.content_height - view;
            let t = (self.scroll_y / max_scroll).clamp(0.0, 1.0);
            self.thumb.set_size(Size::new(4.0, thumb_h));
            self.thumb.set_position(Position::new(
                base.x + self.region.size.width - 6.0,
                base.y + t * (view - thumb_h),
                base.z + 5.0,
            ));
        } else {
            self.thumb.set_size(Size::new(0.0, 0.0));
        }
    }
}

impl ScrollArea {
    pub fn set_size(&mut self, size: Size) {
        self.region.size = size;
        self.frame.set_size(size);
        self.stencil.set_size(&size - (2.0, 2.0));
        let max_scroll = (self.content_height - size.height).max(0.0);
        self.scroll_y = self.scroll_y.min(max_scroll);
    }

    pub fn set_position(&mut self, position: Position) {
        let z = self.region.position.z;
        self.region.position = Position::new(position.x, position.y, z);
    }

    pub fn clear(&mut self) {
        self.children.clear();
        self.scroll_y = 0.0;
    }

    pub fn set_gap(&mut self, gap: f32) {
        self.gap = gap;
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
        crate::ui::clip::push(&self.stencil);
        for child in self.children.values() {
            child.render();
        }
        crate::ui::clip::pop(&self.stencil);
        self.thumb.render();
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
