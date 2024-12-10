use std::collections::BTreeMap;

use crate::core::{
    renderer::{
        plane::{PlaneBuilder, PlaneRenderer},
        ui::{
            primitives::{Position, Region},
            Offset, Size, UIElement, UIElementHandle,
        },
    },
    scene::Scene,
};

use super::{Button, ButtonBuilder};

impl UIElement for Button {
    fn render(&mut self, scene: &mut Scene) {
        PlaneRenderer::render(&self.plane);
        for child in self.children.values_mut() {
            child.render(scene);
        }
    }

    fn set_offset(&mut self, offset: Offset) {
        self.offset = offset;
        self.plane.set_position(&self.position + &self.offset);
        for child in self.children.values_mut() {
            child.set_offset(&self.offset + &self.position);
        }
    }

    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut glfw::Window,
        _: &mut glfw::Glfw,
        event: &glfw::WindowEvent,
    ) -> bool {
        let region = Region::new_with_offset(self.position, self.size, self.offset);
        match event {
            glfw::WindowEvent::MouseButton(glfw::MouseButton::Button1, glfw::Action::Press, _) => {
                let (x, y) = window.get_cursor_pos();
                let (x, y) = (x as f32, y as f32);
                if region.contains(x, y) {
                    (self.on_click)(scene);
                    return true;
                }
                false
            }
            glfw::WindowEvent::CursorPos(x, y) => {
                if region.contains(*x as f32, *y as f32) {
                    if !self.is_hovering {
                        window.set_cursor(Some(glfw::Cursor::standard(glfw::StandardCursor::Hand)));
                        self.is_hovering = true;
                        self.plane.set_color((0.3, 0.4, 0.6, 1.0));
                    }
                } else if self.is_hovering {
                    window.set_cursor(None);
                    self.is_hovering = false;
                    self.plane.set_color((0.2, 0.3, 0.5, 1.0));
                }
                false
            }
            _ => false,
        }
    }

    fn add_children(&mut self, children: Vec<(Option<UIElementHandle>, Box<dyn UIElement>)>) {
        for (handle, mut child) in children {
            child.set_offset(&self.offset + &self.position);
            child.set_z_index(self.position.z + 1.0);
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

impl Button {
    pub fn new(position: Position, size: Size, on_click: Box<dyn Fn(&mut Scene)>) -> Self {
        Self {
            position,
            size,
            on_click,
            children: BTreeMap::new(),
            offset: Offset::default(),
            is_hovering: false,
            plane: PlaneBuilder::new()
                .position(position)
                .size(size)
                .border_radius_uniform(5.0)
                .border_thickness(1.0)
                .color((0.2, 0.3, 0.5, 1.0))
                .build(),
        }
    }
}

impl ButtonBuilder {
    pub fn new() -> Self {
        Self {
            position: Position::default(),
            size: Size::default(),
            on_click: Box::new(|_| {}),
            children: Vec::new(),
        }
    }

    pub fn position(mut self, x: f32, y: f32) -> Self {
        self.position = Position { x, y, z: 0.0 };
        self
    }

    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.size = Size { width, height };
        self
    }

    pub fn on_click(mut self, on_click: Box<dyn Fn(&mut Scene)>) -> Self {
        self.on_click = on_click;
        self
    }

    pub fn add_child(mut self, child: Box<dyn UIElement>) -> Self {
        self.children.push((None, child));
        self
    }

    pub fn build(self) -> Button {
        let mut button = Button::new(self.position, self.size, self.on_click);
        button.add_children(self.children);
        button
    }
}
