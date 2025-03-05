use ferrite::core::{
    primitives::{Offset, Position, Region, Size},
    renderer::plane::PlaneBuilder,
    scene::Scene,
};
use glfw::{Cursor, Glfw, Window, WindowEvent};

use crate::ui::{
    element_handle::UIElementHandle,
    elements::{container::{Container, Direction}, text::Text},
    UIElement,
};

use glfw::{
    Action::{Press, Release},
    MouseButtonLeft,
    StandardCursor::Hand,
    WindowEvent::{CursorPos, MouseButton},
};

use super::Panel;

impl Panel {
    pub fn new(title: String, position: Position, size: Size) -> Self {
        let mut title = Text::new(title, 16.0);
        title.set_z_index(position.z + 2.0);
        let mut content = Container::new((0.0, 0.0, position.z + 1.0).into(), &size - (0.0, 40.0));
        content.set_offset((&position + (0.0, 20.0)).into());
        let mut controls = Container::new(Position::new(size.width - 2.0, -2.0, position.z + 2.0), Size::new(0.0, 20.0));
        controls.set_direction(Direction::Horizontal);
        controls.set_offset(position.into());

        let plane = PlaneBuilder::new()
            .position(position)
            .size(size)
            .color((0.2, 0.2, 0.2, 1.0))
            .border_radius_uniform(5.0)
            .border_thickness(1.0)
            .build();
        let header_plane = PlaneBuilder::new()
            .position(&position + (0.0, 0.0, 1.0))
            .size(Size {
                width: size.width,
                height: 20.0,
            })
            .color((0.2, 0.3, 0.5, 1.0))
            .border_radius((5.0, 5.0, 0.0, 0.0))
            .border_thickness(1.0)
            .build();

        Self {
            handle: UIElementHandle::new(),
            region: Region::new(position, size),

            title,
            title_source: None,
            content,
            controls,

            is_collapsible: false,
            is_movable: true,
            has_controls: false,

            is_hovering: false,
            is_open: true,
            is_dragging: false,
            is_moved: false,

            drag_position: None,

            header_plane,
            plane,
        }
    }

    pub fn close(&mut self) {
        self.is_open = false;
    }

    pub fn set_size(&mut self, size: Size) {
        self.region.size = size;
        self.plane.set_size(size);
        self.header_plane.set_size(Size {
            width: size.width,
            height: if self.has_controls { 24.0 } else { 20.0 },
        });
    }

    pub fn set_handle(&mut self, handle: UIElementHandle) {
        self.handle = handle;
    }

    pub fn set_collapsible(&mut self, collapsible: bool) {
        self.is_collapsible = collapsible;
    }

    pub fn set_movable(&mut self, movable: bool) {
        self.is_movable = movable;
    }

    pub fn set_with_end_gap(&mut self, with_end_gap: bool) {
        self.content.with_end_gap(with_end_gap);
    }

    pub fn add_control(&mut self, control: Box<dyn UIElement>) {
        self.controls.add_child(control);
        self.controls.set_position(Position {
            x: self.region.size.width - self.controls.get_size().width - 2.5,
            y: -2.0,
            z: self.region.position.z + 2.0,
        });
        self.has_controls = true;
        self.header_plane.set_size(Size {
            width: self.region.size.width,
            height: 24.0,
        });
    }
}

impl UIElement for Panel {
    fn update(&mut self, scene: &mut Scene) {
        if let Some(source) = &self.title_source {
            self.title.set_text(source.read());
        }
        self.title.update(scene);
        self.controls.update(scene);
        self.content.update(scene);
    }

    fn render(&self) {
        self.plane.render();
        self.header_plane.render();
        self.title.render();
        self.controls.render();
        if !self.is_collapsible || self.is_open {
            self.content.render();
        }
    }

    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut Window,
        glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        if self.controls.handle_events(scene, window, glfw, event) {
            return true;
        }
        match event {
            MouseButton(MouseButtonLeft, Press, _) => {
                let (x, y) = window.get_cursor_pos();
                let (x, y) = (x as f32, y as f32);
                if self.is_hovering {
                    self.is_dragging = true;
                    if self.is_movable {
                        self.drag_position = Some((x, y));
                        self.is_moved = false;
                    }
                    return true;
                }
            }
            MouseButton(MouseButtonLeft, Release, _) => {
                if self.is_collapsible && !self.is_moved && self.is_dragging {
                    self.is_open = !self.is_open;
                    if self.is_open {
                        let content_size = self.content.get_size();
                        self.header_plane.border_radius = (0.0, 5.0, 0.0, 5.0);
                        self.set_size(content_size + (0.0, 20.0));
                    } else {
                        self.set_size(Size {
                            width: self.region.size.width,
                            height: if self.has_controls { 24.0 } else { 20.0 },
                        });
                        self.header_plane.border_radius = (5.0, 5.0, 5.0, 5.0);
                    }
                }
                self.is_dragging = false;
                self.is_moved = false;
                self.drag_position = None;
            }
            CursorPos(x, y) => {
                let (x, y) = (*x as f32, *y as f32);
                if self.header_plane.get_region().contains(x, y) {
                    if !self.is_hovering {
                        window.set_cursor(Some(Cursor::standard(Hand)));
                        self.is_hovering = true;
                        self.header_plane.set_color((0.3, 0.4, 0.6, 1.0));
                    }
                } else if self.is_hovering {
                    window.set_cursor(None);
                    self.is_hovering = false;
                    self.header_plane.set_color((0.2, 0.3, 0.5, 1.0));
                }
                if self.is_dragging {
                    if let Some((start_x, start_y)) = self.drag_position {
                        let offset = (
                            x - start_x - self.region.offset.x,
                            y - start_y - self.region.offset.y,
                        );
                        self.region.position.x += offset.0;
                        self.region.position.y += offset.1;
                        self.drag_position = Some((x, y));
                        self.is_moved = true;
                        self.set_offset(self.region.offset); // update children
                    }
                }
            }
            _ => (),
        }
        self.content.handle_events(scene, window, glfw, event)
    }

    fn add_child(&mut self, child: Box<dyn UIElement>) {
        self.content.add_child(child);
    }

    fn add_child_to(&mut self, parent: UIElementHandle, child: Box<dyn UIElement>) {
        if self.content.contains_child(&parent) {
            self.content.add_child_to(parent, child);
        }
    }

    fn contains_child(&self, handle: &UIElementHandle) -> bool {
        self.content.contains_child(handle)
    }

    fn get_child(&self, handle: &UIElementHandle) -> Option<&Box<dyn UIElement>> {
        self.content.get_child(handle)
    }

    fn get_child_mut(&mut self, handle: &UIElementHandle) -> Option<&mut Box<dyn UIElement>> {
        self.content.get_child_mut(handle)
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
        self.header_plane.set_position(&(&self.region.position + &self.region.offset) + (0.0, 0.0, 1.0));
        self.title.set_position(&(&self.region.position + &self.region.offset) + (8.0, if self.has_controls { 4.0 } else { 2.0 }, 3.0));
        self.content.set_offset(&self.region.offset + &self.region.position + (0.0, 20.0));
        self.controls.set_offset(&self.region.offset + &self.region.position);
    }

    fn get_size(&self) -> &Size {
        &self.region.size
    }

    fn set_z_index(&mut self, z_index: f32) {
        self.region.position.z = z_index;
        self.plane.set_z_index(z_index);
        self.header_plane.set_z_index(z_index + 1.0);
        self.title.set_z_index(z_index + 2.0);
        self.content.set_z_index(z_index + 1.0);
        self.controls.set_z_index(z_index + 3.0);
    }
}
