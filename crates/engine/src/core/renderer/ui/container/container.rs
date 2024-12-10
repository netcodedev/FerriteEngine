use std::collections::BTreeMap;

use crate::core::{
    renderer::{
        plane::{PlaneBuilder, PlaneRenderer},
        ui::{primitives::Position, Offset, Size, UIElement, UIElementHandle},
    },
    scene::Scene,
};

use super::{Container, ContainerBuilder, Direction};

impl Container {
    pub fn new(position: Position, size: Size) -> Self {
        Self {
            position,
            size,
            children: BTreeMap::new(),
            offset: Offset::default(),
            gap: 5.0,
            plane: PlaneBuilder::new()
                .position(position)
                .size(size)
                .color((0.0, 0.0, 0.0, 0.0))
                .border_color((0.0, 0.0, 0.0, 0.0))
                .build(),
            with_end_gap: true,
            direction: Direction::Vertical,
        }
    }

    pub fn with_end_gap(&mut self, with_end_gap: bool) {
        self.with_end_gap = with_end_gap;
    }

    pub fn set_position(&mut self, position: Position) {
        self.position = position;
        self.plane.set_position(&self.position + &self.offset);
        for child in self.children.values_mut() {
            child.set_offset(&self.offset + &self.position);
        }
    }
}

impl UIElement for Container {
    fn render(&mut self, scene: &mut Scene) {
        PlaneRenderer::render(&self.plane);
        match self.direction {
            Direction::Horizontal => {
                let mut x_offset = self.gap;
                for child in self.children.values_mut() {
                    let offset = &self.offset + &self.position + (x_offset, self.gap);
                    if offset != *child.get_offset() {
                        child.set_offset(offset);
                    }
                    x_offset += child.get_size().width + self.gap;
                    child.render(scene);
                }
                self.size.width = x_offset;
                if !self.with_end_gap {
                    self.size.width -= self.gap;
                }
            }
            Direction::Vertical => {
                let mut y_offset = self.gap;
                for child in self.children.values_mut() {
                    let offset = &self.offset + &self.position + (self.gap, y_offset);
                    if offset != *child.get_offset() {
                        child.set_offset(offset);
                    }
                    y_offset += child.get_size().height + self.gap;
                    child.render(scene);
                }
                self.size.height = y_offset;
                if !self.with_end_gap {
                    self.size.height -= self.gap;
                }
            }
        }
    }

    fn set_offset(&mut self, offset: Offset) {
        self.offset = offset;
        self.plane.set_position(&self.position + &self.offset);
        match self.direction {
            Direction::Horizontal => {
                let mut current_x_offset = self.gap;
                for child in &mut self.children.values_mut() {
                    child.set_offset(&self.offset + &self.position + (current_x_offset, self.gap));
                    current_x_offset += child.get_size().width + self.gap;
                }
            }
            Direction::Vertical => {
                let mut current_y_offset = self.gap;
                for child in &mut self.children.values_mut() {
                    child.set_offset(&self.offset + &self.position + (self.gap, current_y_offset));
                    current_y_offset += child.get_size().height + self.gap;
                }
            }
        }
    }

    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut glfw::Window,
        glfw: &mut glfw::Glfw,
        event: &glfw::WindowEvent,
    ) -> bool {
        // test if click is within bounds
        for child in &mut self.children.values_mut() {
            if child.handle_events(scene, window, glfw, event) {
                return true;
            }
        }
        false
    }

    fn add_children(&mut self, children: Vec<(Option<UIElementHandle>, Box<dyn UIElement>)>) {
        for (handle, mut child) in children {
            child.set_z_index(self.position.z + 1.0);
            match self.direction {
                Direction::Horizontal => {
                    child.set_offset(
                        &(child.get_offset() + &self.offset)
                            + &self.position
                            + (self.size.width, 0.0),
                    );
                    self.size.width += child.get_size().width + self.gap;
                }
                Direction::Vertical => {
                    child.set_offset(
                        &(child.get_offset() + &self.offset)
                            + &self.position
                            + (0.0, self.size.height),
                    );
                    self.size.height += child.get_size().height + self.gap;
                }
            }
            self.plane.set_size(self.size);
            let handle = handle.unwrap_or(UIElementHandle::new());
            self.children.insert(handle, child);
        }
    }

    fn get_size(&self) -> &Size {
        &self.size
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

    fn get_offset(&self) -> &Offset {
        &self.offset
    }

    fn add_child_to(
        &mut self,
        parent: UIElementHandle,
        id: Option<UIElementHandle>,
        element: Box<dyn UIElement>,
    ) {
        if let Some(parent) = self.children.get_mut(&parent) {
            parent.add_children(vec![(id, element)]);
        } else {
            for (_, child) in &mut self.children {
                if child.contains_child(&parent) {
                    child.add_child_to(parent, id, element);
                    return;
                }
            }
        }
    }

    fn set_z_index(&mut self, z_index: f32) {
        self.position.z = z_index;
        self.plane.set_z_index(z_index);
        for child in self.children.values_mut() {
            child.set_z_index(z_index + 1.0);
        }
    }
}

impl ContainerBuilder {
    pub fn new() -> Self {
        Self {
            position: Position::default(),
            size: Size::default(),
            children: Vec::new(),
            with_end_gap: true,
            direction: Direction::Vertical,
        }
    }

    pub fn position(mut self, x: f32, y: f32, z: f32) -> Self {
        self.position = Position { x, y, z };
        self
    }

    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.size = Size { width, height };
        self
    }

    pub fn add_child(mut self, handle: Option<UIElementHandle>, child: Box<dyn UIElement>) -> Self {
        self.children.push((handle, child));
        self
    }

    pub fn with_end_gap(mut self, with_end_gap: bool) -> Self {
        self.with_end_gap = with_end_gap;
        self
    }

    pub fn direction(mut self, direction: Direction) -> Self {
        self.direction = direction;
        self
    }

    pub fn build(self) -> Container {
        let mut container = Container::new(self.position, self.size);
        container.with_end_gap = self.with_end_gap;
        container.direction = self.direction;
        container.add_children(self.children);
        container
    }
}
