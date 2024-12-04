use std::collections::BTreeMap;

use crate::core::{
    renderer::{
        plane::{PlaneBuilder, PlaneRenderer},
        ui::{offset::Offset, position::Position, size::Size, UIElement, UIElementHandle},
    },
    scene::Scene,
};

use super::{Container, ContainerBuilder};

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
        }
    }

    pub fn with_end_gap(&mut self, with_end_gap: bool) {
        self.with_end_gap = with_end_gap;
    }
}

impl UIElement for Container {
    fn render(&mut self, scene: &mut Scene) {
        PlaneRenderer::render(&self.plane);
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

    fn set_offset(&mut self, offset: Offset) {
        self.offset = offset;
        self.plane.set_position(&self.position + &self.offset);
        let mut current_y_offset = self.gap;
        for child in &mut self.children.values_mut() {
            child.set_offset(&self.offset + &self.position + (self.gap, current_y_offset));
            current_y_offset += child.get_size().height + self.gap;
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
        match event {
            glfw::WindowEvent::MouseButton(glfw::MouseButton::Button1, glfw::Action::Press, _) => {
                let (x, y) = window.get_cursor_pos();
                if x as f32 >= self.offset.x + self.position.x
                    && x as f32 <= self.offset.x + self.position.x + self.size.width
                    && y as f32 >= self.offset.y + self.position.y
                    && y as f32 <= self.offset.y + self.position.y + self.size.height
                {
                    for child in &mut self.children.values_mut() {
                        if child.handle_events(scene, window, glfw, event) {
                            return true;
                        }
                    }
                }
            }
            _ => (),
        }
        for child in &mut self.children.values_mut() {
            if child.handle_events(scene, window, glfw, event) {
                return true;
            }
        }
        false
    }

    fn add_children(&mut self, children: Vec<(Option<UIElementHandle>, Box<dyn UIElement>)>) {
        for (handle, mut child) in children {
            let offset = child.get_offset();
            child.set_offset(
                &(offset + &self.offset) + &self.position + (self.gap, self.size.height),
            );
            self.size.height += child.get_size().height + self.gap;
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
}

impl ContainerBuilder {
    pub fn new() -> Self {
        Self {
            position: Position::default(),
            size: Size::default(),
            children: Vec::new(),
            with_end_gap: true,
        }
    }

    pub fn position(mut self, x: f32, y: f32) -> Self {
        self.position = Position { x, y };
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

    pub fn build(self) -> Container {
        let mut container = Container::new(self.position, self.size);
        container.with_end_gap = self.with_end_gap;
        container.add_children(self.children);
        container
    }
}
