use std::collections::BTreeMap;

use ferrite::core::{
    primitives::{Offset, Position, Region, Size},
    renderer::{plane::PlaneBuilder, text::{Fonts, Text}},
    scene::Scene,
};

use crate::ui::{element_handle::UIElementHandle, UIElement};

use glfw::{
    Action::Press,
    Glfw, MouseButtonLeft, Window,
    WindowEvent::{self, CursorPos, MouseButton},
};

use super::Button;

impl Button {
    pub fn new(position: Position, size: Size, on_click: Box<dyn Fn(&mut Scene)>) -> Self {
        Self {
            handle: UIElementHandle::new(),

            region: Region::new_with_offset(position, size, Offset::default()),
            padding: Offset { x: 5.0, y: 1.0 },

            on_click,
            children: BTreeMap::new(),

            is_hovering: false,
            is_active: false,

            label: None,
            plane: PlaneBuilder::new()
                .position(position)
                .size(size)
                .border_radius_uniform(5.0)
                .border_thickness(1.0)
                .color((0.2, 0.3, 0.5, 1.0))
                .build(),
        }
    }

    pub fn set_size(&mut self, size: Size) {
        self.region.size = size;
        self.plane.set_size(size);
    }

    /// Text drawn inside the button. Unlike a `Text` child, it can be
    /// changed later with `set_label` (e.g. "Play" ↔ "Stop").
    pub fn set_label(&mut self, label: &str) {
        match &mut self.label {
            Some((current, _)) if current == label => {}
            Some((current, text)) => {
                *current = label.to_owned();
                text.set_content(label);
            }
            None => {
                let mut text = Text::new(Fonts::RobotoMono, 0, 0, 0, 15.0, label.to_owned());
                text.set_z_index(self.region.position.z + 1.0);
                self.label = Some((label.to_owned(), text));
            }
        }
    }

    /// Highlighted look for toggle-style buttons (selected tool, play mode).
    pub fn set_active(&mut self, active: bool) {
        if self.is_active != active {
            self.is_active = active;
            self.plane.set_color(self.base_color());
        }
    }

    fn base_color(&self) -> (f32, f32, f32, f32) {
        match (self.is_active, self.is_hovering) {
            (true, _) => (0.35, 0.5, 0.25, 1.0),
            (false, true) => (0.3, 0.4, 0.6, 1.0),
            (false, false) => (0.2, 0.3, 0.5, 1.0),
        }
    }
}

impl UIElement for Button {
    fn update(&mut self, scene: &mut Scene) {
        let origin = self.region.get_absolute_position();
        let size = self.region.size;
        if let Some((_, text)) = &mut self.label {
            let text_size = text.get_size();
            let x = (size.width - text.measure_width()).max(0.0) * 0.5;
            let y = (size.height - text_size.height).max(0.0) * 0.5;
            text.prepare_render_at(&origin + (x, y, 1.0));
        }
        for child in self.children.values_mut() {
            child.update(scene);
        }
    }

    fn render(&self) {
        self.plane.render();
        if let Some((_, text)) = &self.label {
            text.render();
        }
        for child in self.children.values() {
            child.render();
        }
    }

    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut Window,
        _glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        match event {
            MouseButton(MouseButtonLeft, Press, _) => {
                let (x, y) = window.get_cursor_pos();
                let (x, y) = (x as f32, y as f32);
                if self.region.contains(x, y) {
                    (self.on_click)(scene);
                    return true;
                }
                false
            }
            CursorPos(x, y) => {
                if self.region.contains(*x as f32, *y as f32) {
                    if !self.is_hovering {
                        window.set_cursor(Some(glfw::Cursor::standard(glfw::StandardCursor::Hand)));
                        self.is_hovering = true;
                        self.plane.set_color(self.base_color());
                    }
                } else if self.is_hovering {
                    window.set_cursor(None);
                    self.is_hovering = false;
                    self.plane.set_color(self.base_color());
                }
                false
            }
            _ => false,
        }
    }

    fn add_child(&mut self, child: Box<dyn UIElement>) {
        self.children.insert(child.get_handle().clone(), child);
    }

    fn add_child_to(&mut self, parent: UIElementHandle, child: Box<dyn UIElement>) {
        if let Some(parent) = self.children.get_mut(&parent) {
            parent.add_child(child);
        } else {
            for child_elem in self.children.values_mut() {
                if child_elem.contains_child(&parent) {
                    child_elem.add_child_to(parent, child);
                    return;
                }
            }
        }
    }

    fn contains_child(&self, handle: &UIElementHandle) -> bool {
        if self.children.contains_key(handle) {
            return true;
        }
        for child in self.children.values() {
            if child.contains_child(handle) {
                return true;
            }
        }
        false
    }

    fn get_child(&self, handle: &UIElementHandle) -> Option<&Box<dyn UIElement>> {
        if let Some(child) = self.children.get(handle) {
            return Some(child);
        }
        for child in self.children.values() {
            if let Some(child) = child.get_child(handle) {
                return Some(child);
            }
        }
        None
    }

    fn get_child_mut(&mut self, handle: &UIElementHandle) -> Option<&mut Box<dyn UIElement>> {
        for (child_handle, child) in self.children.iter_mut() {
            if child_handle == handle {
                return Some(child);
            }
            if let Some(child) = child.get_child_mut(handle) {
                return Some(child);
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
        self.plane.set_position(&self.region.position + &self.region.offset);
        for child in self.children.values_mut() {
            child.set_offset(&(&self.region.offset + &self.padding) + &self.region.position);
        }
    }

    fn get_size(&self) -> &Size {
        &self.region.size
    }

    fn set_z_index(&mut self, z_index: f32) {
        self.region.position.z = z_index;
        self.plane.set_z_index(z_index);
        if let Some((_, text)) = &mut self.label {
            text.set_z_index(z_index + 1.0);
        }
        for child in self.children.values_mut() {
            child.set_z_index(z_index + 1.0);
        }
    }
}
