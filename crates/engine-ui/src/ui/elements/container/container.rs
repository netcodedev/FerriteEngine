use std::collections::BTreeMap;

use ferrite::core::{
    primitives::{Offset, Position, Region, Size},
    renderer::plane::PlaneBuilder,
    scene::Scene,
};
use glfw::{Glfw, Window, WindowEvent};

use crate::ui::{element_handle::UIElementHandle, UIElement};

use super::{Container, Direction};

impl Container {
    pub fn new(position: Position, size: Size) -> Self {
        Self {
            handle: UIElementHandle::new(),
            region: Region::new(position, size),
            children: BTreeMap::new(),

            gap: 5.0,
            direction: Direction::Vertical,
            with_end_gap: true,

            plane: PlaneBuilder::new()
                .position(position)
                .size(size)
                .color((0.0, 0.0, 0.0, 0.0))
                .border_color((0.0, 0.0, 0.0, 0.0))
                .build(),
        }
    }

    pub fn with_end_gap(&mut self, with_end_gap: bool) {
        self.with_end_gap = with_end_gap;
    }

    pub fn set_direction(&mut self, direction: Direction) {
        self.direction = direction;
    }

    pub fn set_position(&mut self, position: Position) {
        self.region.position = position;
        self.plane
            .set_position(&self.region.position + &self.region.offset);
        for child in self.children.values_mut() {
            child.set_offset(&self.region.offset + &self.region.position);
        }
    }
}

impl UIElement for Container {
    fn update(&mut self, scene: &mut Scene) {
        match self.direction {
            Direction::Horizontal => {
                let mut x_offset = self.gap;
                for child in self.children.values_mut() {
                    let offset = &self.region.offset + &self.region.position + (x_offset, self.gap);
                    if offset != *child.get_offset() {
                        child.set_offset(offset);
                    }
                    x_offset += child.get_size().width + self.gap;
                    child.update(scene);
                }
                self.region.size.width = x_offset;
                if !self.with_end_gap {
                    self.region.size.width -= self.gap;
                }
            }
            Direction::Vertical => {
                let mut y_offset = self.gap;
                for child in self.children.values_mut() {
                    let offset = &self.region.offset + &self.region.position + (self.gap, y_offset);
                    if offset != *child.get_offset() {
                        child.set_offset(offset);
                    }
                    y_offset += child.get_size().height + self.gap;
                    child.update(scene);
                }
                self.region.size.height = y_offset;
                if !self.with_end_gap {
                    self.region.size.height -= self.gap;
                }
            }
        }
        for child in self.children.values_mut() {
            child.update(scene);
        }
    }

    fn render(&self) {
        self.plane.render();
        for child in self.children.values() {
            child.render();
        }
    }

    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut Window,
        glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        for child in self.children.values_mut() {
            if child.handle_events(scene, window, glfw, event) {
                return true;
            }
        }
        false
    }

    fn add_child(&mut self, mut child: Box<dyn UIElement>) {
        child.set_z_index(self.region.position.z + 1.0);
        match self.direction {
            Direction::Horizontal => {
                child.set_offset(
                    &(child.get_offset() + &self.region.offset)
                        + &self.region.position
                        + (self.region.size.width, 0.0),
                );
                self.region.size.width += child.get_size().width + self.gap;
            }
            Direction::Vertical => {
                child.set_offset(
                    &(child.get_offset() + &self.region.offset)
                        + &self.region.position
                        + (0.0, self.gap),
                );
                self.region.size.height += child.get_size().height + self.gap;
            }
        }
        self.plane.set_size(self.region.size);
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
        match self.direction {
            Direction::Horizontal => {
                let mut current_x_offset = self.gap;
                for child in &mut self.children.values_mut() {
                    child.set_offset(&self.region.offset + &self.region.position + (current_x_offset, self.gap));
                    current_x_offset += child.get_size().width + self.gap;
                }
            }
            Direction::Vertical => {
                let mut current_y_offset = self.gap;
                for child in &mut self.children.values_mut() {
                    child.set_offset(&self.region.offset + &self.region.position + (self.gap, current_y_offset));
                    current_y_offset += child.get_size().height + self.gap;
                }
            }
        }
    }

    fn get_size(&self) -> &Size {
        &self.region.size
    }

    fn set_z_index(&mut self, z_index: f32) {
        self.region.position.z = z_index;
        self.plane.set_z_index(z_index);
        for child in self.children.values_mut() {
            child.set_z_index(z_index + 1.0);
        }
    }
}
