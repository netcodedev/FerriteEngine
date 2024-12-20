use crate::core::{
    renderer::{
        plane::{PlaneBuilder, PlaneRenderer},
        text::{Fonts, Text},
        ui::{
            container::{ContainerBuilder, Direction},
            primitives::{Position, Region},
            Offset, Size, UIElement, UIElementHandle,
        },
    },
    scene::Scene,
    utils::DataSource,
};

use super::{Panel, PanelBuilder};

impl UIElement for Panel {
    fn render(&mut self, scene: &mut Scene) {
        if !self.collapsible || self.is_open {
            let content_size = self.content.get_size();
            self.header_plane.border_radius = (0.0, 5.0, 0.0, 5.0);
            self.set_size(content_size + (0.0, 20.0));
        } else if self.collapsible && !self.is_open {
            self.set_size(Size {
                width: self.size.width,
                height: if self.has_controls { 24.0 } else { 20.0 },
            });
            self.header_plane.border_radius = (5.0, 5.0, 5.0, 5.0);
        }
        PlaneRenderer::render(&self.plane);
        PlaneRenderer::render(&self.header_plane);
        if let Some(source) = &self.title_source {
            let title = source.read();
            self.title = title.clone();
        }
        self.text.set_content(&self.title);
        self.text.render_at(
            &(&self.position + &self.offset)
                + (8.0, if self.has_controls { 4.0 } else { 2.0 }, 3.0),
        );
        self.controls.render(scene);
        if !self.collapsible || self.is_open {
            self.content.render(scene);
        }
    }

    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut glfw::Window,
        glfw: &mut glfw::Glfw,
        event: &glfw::WindowEvent,
    ) -> bool {
        if self.controls.handle_events(scene, window, glfw, event) {
            return true;
        }
        // test if click is within bounds
        match event {
            glfw::WindowEvent::MouseButton(glfw::MouseButton::Button1, glfw::Action::Press, _) => {
                let (x, y) = window.get_cursor_pos();
                let region = Region::new_with_offset(
                    self.position,
                    Size {
                        width: self.size.width,
                        height: 20.0,
                    },
                    self.offset,
                );
                if region.contains(x as f32, y as f32) {
                    // Start dragging
                    self.dragging = true;
                    if self.movable {
                        self.drag_start = Some(Position {
                            x: x as f32,
                            y: y as f32,
                            z: self.position.z,
                        });
                    }
                    self.moved = false;
                    return true;
                }
            }
            glfw::WindowEvent::MouseButton(
                glfw::MouseButton::Button1,
                glfw::Action::Release,
                _,
            ) => {
                // Stop dragging
                if self.collapsible && !self.moved && self.dragging {
                    self.is_open = !self.is_open;
                    if self.is_open {
                        self.plane.set_size(Size {
                            width: 100.0,
                            height: 100.0,
                        });
                    }
                }
                self.dragging = false;
                self.drag_start = None;
                self.moved = false;
            }
            glfw::WindowEvent::CursorPos(x, y) => {
                let (x, y) = (*x as f32, *y as f32);
                let region = Region::new_with_offset(
                    self.position,
                    Size {
                        width: self.size.width,
                        height: 20.0,
                    },
                    self.offset,
                );
                if region.contains(x, y) {
                    if !self.is_hovering {
                        window.set_cursor(Some(glfw::Cursor::standard(glfw::StandardCursor::Hand)));
                        self.is_hovering = true;
                        self.header_plane.set_color((0.3, 0.4, 0.6, 1.0));
                    }
                } else if self.is_hovering {
                    window.set_cursor(None);
                    self.is_hovering = false;
                    self.header_plane.set_color((0.2, 0.3, 0.5, 1.0));
                }
                if self.dragging {
                    // Update panel position while dragging
                    if let Some(position) = self.drag_start {
                        self.position.x += x - position.x - self.offset.x;
                        self.position.y += y - position.y - self.offset.y;
                        self.drag_start = Some(Position {
                            x,
                            y,
                            z: position.z,
                        });
                        self.moved = true;
                        self.set_offset(self.offset); // update children
                    }
                    return true;
                }
            }
            _ => (),
        }
        self.content.handle_events(scene, window, glfw, event)
    }

    fn add_children(&mut self, children: Vec<(Option<UIElementHandle>, Box<dyn UIElement>)>) {
        self.content.add_children(children);
    }

    fn set_offset(&mut self, offset: Offset) {
        self.offset = offset;
        self.plane.set_position(&self.position + &self.offset);
        self.header_plane
            .set_position(&(&self.position + &self.offset) + (0.0, 0.0, 1.0));
        self.text
            .set_z_index((&(&self.position + &self.offset) + (0.0, 0.0, 2.0)).z);
        self.controls.set_offset(&self.offset + &self.position);
        self.content.set_offset(Offset {
            x: self.offset.x + self.position.x,
            y: self.offset.y + self.position.y + 20.0,
        });
    }

    fn get_size(&self) -> &Size {
        &self.size
    }

    fn contains_child(&self, handle: &UIElementHandle) -> bool {
        self.content.contains_child(handle)
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
        if let Some(parent) = self.content.children.get_mut(&parent) {
            parent.add_children(vec![(id, element)]);
        } else {
            for (_, child) in &mut self.content.children {
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
        self.header_plane.set_z_index(z_index + 1.0);
        self.text.set_z_index(z_index + 2.0);
        self.content.set_z_index(z_index + 1.0);
        self.controls.set_z_index(z_index + 3.0);
    }
}

impl Panel {
    pub fn new(title: String, position: Position, size: Size) -> Self {
        let mut content = ContainerBuilder::new()
            .position(0.0, 0.0, &position.z + 1.0)
            .size(size.width, size.height - 40.0)
            .build();
        content.set_offset((&position + (0.0, 20.0)).into());
        let mut controls = ContainerBuilder::new()
            .position(size.width - 2.0, -2.0, &position.z + 1.0)
            .size(0.0, 20.0)
            .direction(Direction::Horizontal)
            .build();
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
        let mut panel = Self {
            position,
            size,
            title: title.clone(),
            title_source: None,
            content,
            controls,
            text: Text::new(Fonts::RobotoMono, 0, 0, 0, 16.0, title),
            offset: Offset::default(),
            drag_start: None,
            dragging: false,
            is_hovering: false,
            plane,
            header_plane,
            collapsible: false,
            movable: true,
            is_open: true,
            moved: false,
            has_controls: false,
        };
        panel.set_z_index(position.z);
        panel
    }

    pub fn set_size(&mut self, size: Size) {
        self.size = size;
        self.plane.set_size(size);
        self.header_plane.set_size(Size {
            width: size.width,
            height: if self.has_controls { 24.0 } else { 20.0 },
        });
    }

    pub fn add_controls(&mut self, controls: Vec<(Option<UIElementHandle>, Box<dyn UIElement>)>) {
        self.controls.add_children(controls);
        self.controls.set_position(Position {
            x: self.size.width - self.controls.get_size().width - 2.5,
            y: -2.0,
            z: self.position.z + 1.0,
        });
        self.has_controls = true;
    }
}

impl PanelBuilder {
    pub fn new(title: &str) -> Self {
        Self {
            position: Position::default(),
            size: Size::default(),
            title: title.to_string(),
            title_source: None,
            children: Vec::new(),
            controls: Vec::new(),
            collapsible: false,
            movable: true,
            open: true,
            with_end_gap: true,
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

    pub fn collapsible(mut self) -> Self {
        self.collapsible = !self.collapsible;
        self
    }

    pub fn closed(mut self) -> Self {
        self.open = false;
        self
    }

    pub fn open(mut self) -> Self {
        self.open = true;
        self
    }

    pub fn movable(mut self, movable: bool) -> Self {
        self.movable = movable;
        self
    }

    pub fn title_source(mut self, source: DataSource<String>) -> Self {
        self.title_source = Some(source);
        self
    }

    pub fn add_child(mut self, id: Option<UIElementHandle>, child: Box<dyn UIElement>) -> Self {
        self.children.push((id, child));
        self
    }

    pub fn add_control(mut self, id: Option<UIElementHandle>, control: Box<dyn UIElement>) -> Self {
        self.controls.push((id, control));
        self
    }

    pub fn with_end_gap(mut self, with_end_gap: bool) -> Self {
        self.with_end_gap = with_end_gap;
        self
    }

    pub fn build(self) -> Panel {
        let mut panel = Panel::new(self.title.clone(), self.position, self.size);
        panel.title_source = self.title_source;
        panel.collapsible = self.collapsible;
        panel.movable = self.movable;
        panel.is_open = self.open;
        panel.content.with_end_gap(self.with_end_gap);
        panel.add_children(self.children);
        panel.add_controls(self.controls);
        panel
    }
}
