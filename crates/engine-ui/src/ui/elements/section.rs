use ferrite::core::{
    primitives::{Offset, Position, Region, Size},
    renderer::{
        plane::{Plane, PlaneBuilder},
        text::{icons, Text},
    },
    scene::Scene,
    utils::DataSource,
};
use glfw::{
    Action::Press,
    Glfw, MouseButtonLeft, Window,
    WindowEvent::{self, CursorPos, MouseButton},
};

use crate::ui::{element_handle::UIElementHandle, theme, UIElement};

const HEADER_HEIGHT: f32 = 28.0;
const GAP: f32 = 6.0;
const INDENT: f32 = 8.0;

/// Collapsible group: a header row (chevron, optional icon, title, optional
/// widget on the right such as a button) above a vertical stack of children.
pub struct Section {
    handle: UIElementHandle,
    offset: Offset,
    z: f32,
    size: Size,
    width: f32,
    open: bool,
    hovering: bool,
    header_bg: Plane,
    chevron: Text,
    icon: Option<Text>,
    title: Text,
    accessory: Option<Box<dyn UIElement>>,
    children: Vec<Box<dyn UIElement>>,
    /// Shared open/closed flag, so the state survives when the owner
    /// rebuilds its sections.
    open_flag: Option<DataSource<bool>>,
}

impl Section {
    pub fn new(title: &str, icon: Option<&str>, width: f32) -> Self {
        let mut section = Self {
            handle: UIElementHandle::new(),
            offset: Offset::default(),
            z: 0.0,
            size: Size::new(width, HEADER_HEIGHT),
            width,
            open: true,
            hovering: false,
            header_bg: PlaneBuilder::new()
                .color(theme::TRANSPARENT)
                .border_radius_uniform(theme::RADIUS_SMALL)
                .build(),
            chevron: theme::icon(icons::CHEVRON_DOWN, 14.0, theme::TEXT_DIM),
            icon: icon.map(|i| theme::icon(i, 15.0, theme::ACCENT_TEXT)),
            title: theme::heading(title, theme::FONT_SIZE_SMALL + 0.5, theme::TEXT),
            accessory: None,
            children: Vec::new(),
            open_flag: None,
        };
        section.layout();
        section
    }

    /// Widget shown at the right end of the header (e.g. a remove button).
    pub fn set_accessory(&mut self, accessory: Box<dyn UIElement>) {
        self.accessory = Some(accessory);
    }

    pub fn set_width(&mut self, width: f32) {
        self.width = width;
    }

    /// Keep the open/closed state in `flag` (read now and on every update).
    pub fn bind_open(&mut self, flag: DataSource<bool>) {
        let open = flag.read();
        self.open_flag = Some(flag);
        self.set_open(open);
    }

    pub fn set_open(&mut self, open: bool) {
        self.open = open;
        if let Some(flag) = &self.open_flag {
            if flag.read() != open {
                flag.write(open);
            }
        }
        self.chevron.set_content(if open {
            icons::CHEVRON_DOWN
        } else {
            icons::CHEVRON_RIGHT
        });
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn clear(&mut self) {
        self.children.clear();
    }

    fn header_region(&self) -> Region {
        Region::new(
            Position::new(self.offset.x, self.offset.y, self.z),
            Size::new(self.width, HEADER_HEIGHT),
        )
    }

    fn layout(&mut self) {
        let x0 = self.offset.x;
        let y0 = self.offset.y;
        let z = self.z;
        self.header_bg.set_position(Position::new(x0, y0, z));
        self.header_bg
            .set_size(Size::new(self.width, HEADER_HEIGHT));
        self.header_bg.set_color(if self.hovering {
            theme::BG_ROW_HOVER
        } else {
            theme::TRANSPARENT
        });

        let mut x = x0 + 6.0;
        let cy = theme::center_y(&self.chevron, HEADER_HEIGHT);
        self.chevron
            .prepare_render_at(Position::new(x, y0 + cy, z + 1.0));
        x += 18.0;
        if let Some(icon) = &mut self.icon {
            let iy = theme::center_y(icon, HEADER_HEIGHT);
            icon.prepare_render_at(Position::new(x, y0 + iy, z + 1.0));
            x += 22.0;
        }
        let ty = theme::center_y(&self.title, HEADER_HEIGHT);
        self.title
            .prepare_render_at(Position::new(x, y0 + ty, z + 1.0));
        if let Some(accessory) = &mut self.accessory {
            let size = *accessory.get_size();
            let target = Offset {
                x: x0 + self.width - size.width - 4.0,
                y: y0 + (HEADER_HEIGHT - size.height) * 0.5,
            };
            if *accessory.get_offset() != target {
                accessory.set_offset(target);
            }
        }

        let mut y = y0 + HEADER_HEIGHT + if self.open { 2.0 } else { 0.0 };
        if self.open {
            for child in self.children.iter_mut() {
                let target = Offset {
                    x: x0 + INDENT,
                    y,
                };
                if *child.get_offset() != target {
                    child.set_offset(target);
                }
                y += child.get_size().height + GAP;
            }
            y += 2.0;
        }
        self.size = Size::new(self.width, y - y0);
    }
}

impl UIElement for Section {
    fn update(&mut self, scene: &mut Scene) {
        if let Some(open) = self.open_flag.as_ref().map(|f| f.read()) {
            if open != self.open {
                self.set_open(open);
            }
        }
        self.layout();
        if let Some(accessory) = &mut self.accessory {
            accessory.update(scene);
        }
        if self.open {
            for child in self.children.iter_mut() {
                child.update(scene);
            }
        }
    }

    fn render(&self) {
        self.header_bg.render();
        self.chevron.render();
        if let Some(icon) = &self.icon {
            icon.render();
        }
        self.title.render();
        if let Some(accessory) = &self.accessory {
            accessory.render();
        }
        if self.open {
            for child in &self.children {
                child.render();
            }
        }
    }

    fn render_overlay(&self) {
        if let Some(accessory) = &self.accessory {
            accessory.render_overlay();
        }
        if self.open {
            for child in &self.children {
                child.render_overlay();
            }
        }
    }

    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut Window,
        glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        if let Some(accessory) = &mut self.accessory {
            if accessory.handle_events(scene, window, glfw, event)
                && !matches!(event, CursorPos(..))
            {
                return true;
            }
        }
        match event {
            MouseButton(MouseButtonLeft, Press, _) => {
                let (x, y) = window.get_cursor_pos();
                if self.header_region().contains(x as f32, y as f32) {
                    self.set_open(!self.open);
                    return true;
                }
            }
            CursorPos(x, y) => {
                self.hovering = self.header_region().contains(*x as f32, *y as f32);
            }
            _ => {}
        }
        if self.open {
            let mut handled = false;
            for child in self.children.iter_mut() {
                if child.handle_events(scene, window, glfw, event) {
                    handled = true;
                    if !matches!(event, CursorPos(..)) {
                        return true;
                    }
                }
            }
            return handled;
        }
        false
    }

    fn add_child(&mut self, mut child: Box<dyn UIElement>) {
        child.set_z_index(self.z + 1.0);
        self.children.push(child);
    }
    fn add_child_to(&mut self, parent: UIElementHandle, child: Box<dyn UIElement>) {
        if let Some(c) = self.children.iter_mut().find(|c| c.contains_child(&parent)) {
            c.add_child_to(parent, child);
        }
    }
    fn contains_child(&self, handle: &UIElementHandle) -> bool {
        self.children
            .iter()
            .any(|c| c.get_handle() == handle || c.contains_child(handle))
    }
    fn get_child(&self, handle: &UIElementHandle) -> Option<&Box<dyn UIElement>> {
        for c in &self.children {
            if c.get_handle() == handle {
                return Some(c);
            }
            if let Some(x) = c.get_child(handle) {
                return Some(x);
            }
        }
        None
    }
    fn get_child_mut(&mut self, handle: &UIElementHandle) -> Option<&mut Box<dyn UIElement>> {
        for c in self.children.iter_mut() {
            if c.get_handle() == handle {
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
        &self.offset
    }
    fn set_offset(&mut self, offset: Offset) {
        self.offset = offset;
        self.layout();
    }
    fn get_size(&self) -> &Size {
        &self.size
    }
    fn set_z_index(&mut self, z_index: f32) {
        self.z = z_index;
        self.header_bg.set_z_index(z_index);
        self.chevron.set_z_index(z_index + 1.0);
        if let Some(icon) = &mut self.icon {
            icon.set_z_index(z_index + 1.0);
        }
        self.title.set_z_index(z_index + 1.0);
        if let Some(accessory) = &mut self.accessory {
            accessory.set_z_index(z_index + 1.0);
        }
        for child in self.children.iter_mut() {
            child.set_z_index(z_index + 1.0);
        }
    }
}
