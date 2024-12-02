use crate::core::{
    renderer::{
        plane::{PlaneBuilder, PlaneRenderer},
        text::{Fonts, Text},
        ui::{
            container::ContainerBuilder,
            primitives::{Offset, Position, Size},
            UIElement, UIElementHandle,
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
            self.set_size(Size {
                width: content_size.width,
                height: content_size.height + 20.0,
            });
        } else if self.collapsible {
            self.set_size(Size {
                width: self.size.width,
                height: 20.0,
            });
            self.header_plane.border_radius = (5.0, 5.0, 5.0, 5.0);
        }
        PlaneRenderer::render(&self.plane);
        PlaneRenderer::render(&self.header_plane);
        if let Some(source) = &self.title_source {
            let title = source.read();
            self.title = title.clone();
        }
        self.text.set_content(self.title.clone());
        self.text.render_at(
            (self.offset.x + self.position.x + 8.0) as i32,
            (self.offset.y + self.position.y + 2.0) as i32,
        );
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
        // test if click is within bounds
        match event {
            glfw::WindowEvent::MouseButton(glfw::MouseButton::Button1, glfw::Action::Press, _) => {
                let (x, y) = window.get_cursor_pos();
                if x as f32 >= self.offset.x + self.position.x
                    && x as f32 <= self.offset.x + self.position.x + self.size.width
                    && y as f32 >= self.offset.y + self.position.y
                    && y as f32 <= self.offset.y + self.position.y + 20.0
                {
                    // Start dragging
                    self.dragging = true;
                    if self.movable {
                        self.drag_start = Some(Position {
                            x: x as f32,
                            y: y as f32,
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
                if *x as f32 >= self.offset.x + self.position.x
                    && *x as f32 <= self.offset.x + self.position.x + self.size.width
                    && *y as f32 >= self.offset.y + self.position.y
                    && *y as f32 <= self.offset.y + self.position.y + 20.0
                {
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
                        self.position.x += (*x as f32 - position.x) as f32 - self.offset.x;
                        self.position.y += (*y as f32 - position.y) as f32 - self.offset.y;
                        self.drag_start = Some(Position {
                            x: *x as f32,
                            y: *y as f32,
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
        for (handle, child) in children {
            self.content.add_children(vec![(handle, child)]);
        }
    }

    fn set_offset(&mut self, offset: Offset) {
        self.offset = offset;
        self.plane.set_position(self.position + &self.offset);
        self.header_plane.set_position(self.position + &self.offset);
        self.content.set_offset(Offset {
            x: self.offset.x + self.position.x,
            y: self.offset.y + self.position.y + 20.0,
        });
    }

    fn get_size(&self) -> Size {
        self.size
    }

    fn contains_child(&self, handle: &UIElementHandle) -> bool {
        self.content.contains_child(handle)
    }

    fn get_offset(&self) -> Offset {
        self.offset
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
}

impl Panel {
    pub fn new(title: String, position: Position, size: Size) -> Self {
        let mut content = ContainerBuilder::new()
            .position(0.0, 0.0)
            .size(size.width, size.height - 40.0)
            .build();
        content.set_offset(Offset {
            x: position.x,
            y: position.y + 20.0,
        });
        let plane = PlaneBuilder::new()
            .position(position)
            .size(size)
            .color((0.2, 0.2, 0.2, 1.0))
            .border_radius_uniform(5.0)
            .border_thickness(1.0)
            .build();
        let header_plane = PlaneBuilder::new()
            .position(position)
            .size(Size {
                width: size.width,
                height: 20.0,
            })
            .color((0.2, 0.3, 0.5, 1.0))
            .border_radius((5.0, 5.0, 0.0, 0.0))
            .border_thickness(1.0)
            .build();
        Self {
            position,
            size,
            title: title.clone(),
            title_source: None,
            content,
            text: Text::new(Fonts::RobotoMono, 0, 0, 16.0, title),
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
        }
    }

    pub fn set_size(&mut self, size: Size) {
        self.size = size;
        self.plane.set_size(size);
        self.header_plane.set_size(Size {
            width: size.width,
            height: 20.0,
        });
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
            collapsible: false,
            movable: true,
            open: true,
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

    pub fn build(self) -> Panel {
        let mut panel = Panel::new(self.title.clone(), self.position, self.size);
        panel.title_source = self.title_source;
        panel.collapsible = self.collapsible;
        panel.movable = self.movable;
        panel.is_open = self.open;
        panel.add_children(self.children);
        panel
    }
}
