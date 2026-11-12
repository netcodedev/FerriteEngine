use crate::ui::theme;
use ferrite::core::{
    primitives::{Offset, Position, Region, Size},
    renderer::{
        plane::{Plane, PlaneBuilder},
        text::{Fonts, Text as FerriteText},
    },
    scene::Scene,
    utils::DataSource,
};
use glfw::{
    Action::Press, Glfw, MouseButtonLeft, Window,
    WindowEvent::{self, CursorPos, MouseButton},
};

use crate::ui::{element_handle::UIElementHandle, UIElement};

struct TabBtn {
    label: String,
    text: FerriteText,
    bg: Plane,
    region: Region,
}

/// Tab bar bound to `DataSource<u32>`. `pages[i]` is shown when `selected == i`.
pub struct Tabs {
    handle: UIElementHandle,
    region: Region,
    selected: DataSource<u32>,
    bar_height: f32,
    tabs: Vec<TabBtn>,
    pages: Vec<Box<dyn UIElement>>,
    frame: Plane,
    is_hovering_bar: bool,
}

impl Tabs {
    pub fn new(
        position: Position,
        size: Size,
        labels: Vec<String>,
        pages: Vec<Box<dyn UIElement>>,
        selected: DataSource<u32>,
    ) -> Self {
        assert_eq!(labels.len(), pages.len(), "labels and pages must match");
        let bar_height = 22.0;
        let tab_w = if labels.is_empty() {
            0.0
        } else {
            size.width / labels.len() as f32
        };
        let frame = PlaneBuilder::new()
            .position(&position + (0.0, bar_height))
            .size(Size::new(size.width, size.height - bar_height))
            .color(theme::BG_FIELD)
            .border_color(theme::BORDER)
            .border_thickness(1.0)
            .border_radius((0.0, 3.0, 3.0, 3.0))
            .build();
        let tabs = labels
            .into_iter()
            .enumerate()
            .map(|(i, label)| {
                let p = &position + (i as f32 * tab_w, 0.0, 1.0);
                let bg = PlaneBuilder::new()
                    .position(p)
                    .size(Size::new(tab_w, bar_height))
                    .color(theme::BG_CONTROL)
                    .border_color(theme::BORDER)
                    .border_thickness(1.0)
                    .border_radius((3.0, 3.0, 0.0, 0.0))
                    .build();
                let text = FerriteText::new(Fonts::Inter, 0, 0, 0, 14.0, label.clone());
                TabBtn {
                    label,
                    text,
                    bg,
                    region: Region::new(p, Size::new(tab_w, bar_height)),
                }
            })
            .collect();
        Self {
            handle: UIElementHandle::new(),
            region: Region::new(position, size),
            selected,
            bar_height,
            tabs,
            pages,
            frame,
            is_hovering_bar: false,
        }
    }
}

impl UIElement for Tabs {
    fn update(&mut self, scene: &mut Scene) {
        let base = &self.region.position + &self.region.offset;
        let sel = self.selected.read() as usize;
        let tab_w = if self.tabs.is_empty() {
            0.0
        } else {
            self.region.size.width / self.tabs.len() as f32
        };
        for (i, t) in self.tabs.iter_mut().enumerate() {
            let p = &base + (i as f32 * tab_w, 0.0, 1.0);
            t.region.position = p;
            t.bg.set_position(p);
            t.bg.set_color(if i == sel {
                theme::ACCENT_SOFT
            } else {
                theme::BG_CONTROL
            });
            t.text.set_content(&t.label);
            t.text.prepare_render_at(&p + (6.0, 4.0, 1.0));
        }
        self.frame.set_position(&base + (0.0, self.bar_height));
        if let Some(page) = self.pages.get_mut(sel) {
            page.set_offset(Offset {
                x: base.x,
                y: base.y + self.bar_height,
            });
            page.update(scene);
        }
    }

    fn render(&self) {
        self.frame.render();
        for t in &self.tabs {
            t.bg.render();
            t.text.render();
        }
        let sel = self.selected.read() as usize;
        if let Some(page) = self.pages.get(sel) {
            page.render();
        }
    }

    fn render_overlay(&self) {
        let sel = self.selected.read() as usize;
        if let Some(page) = self.pages.get(sel) {
            page.render_overlay();
        }
    }

    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut Window,
        glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        match event {
            MouseButton(MouseButtonLeft, Press, _) => {
                let (x, y) = window.get_cursor_pos();
                let (x, y) = (x as f32, y as f32);
                for (i, t) in self.tabs.iter().enumerate() {
                    if t.region.contains(x, y) {
                        self.selected.write(i as u32);
                        return true;
                    }
                }
            }
            CursorPos(x, y) => {
                let (x, y) = (*x as f32, *y as f32);
                let over = self.tabs.iter().any(|t| t.region.contains(x, y));
                if over && !self.is_hovering_bar {
                    window.set_cursor(Some(glfw::Cursor::standard(glfw::StandardCursor::Hand)));
                    self.is_hovering_bar = true;
                } else if !over && self.is_hovering_bar {
                    window.set_cursor(None);
                    self.is_hovering_bar = false;
                }
            }
            _ => {}
        }
        let sel = self.selected.read() as usize;
        if let Some(page) = self.pages.get_mut(sel) {
            return page.handle_events(scene, window, glfw, event);
        }
        false
    }

    fn add_child(&mut self, child: Box<dyn UIElement>) {
        self.pages.push(child);
    }
    fn add_child_to(&mut self, parent: UIElementHandle, child: Box<dyn UIElement>) {
        for page in &mut self.pages {
            if page.contains_child(&parent) {
                page.add_child_to(parent, child);
                return;
            }
        }
    }
    fn contains_child(&self, handle: &UIElementHandle) -> bool {
        self.pages
            .iter()
            .any(|p| p.get_handle() == handle || p.contains_child(handle))
    }
    fn get_child(&self, handle: &UIElementHandle) -> Option<&Box<dyn UIElement>> {
        for p in &self.pages {
            if p.get_handle() == handle {
                return Some(p);
            }
            if let Some(c) = p.get_child(handle) {
                return Some(c);
            }
        }
        None
    }
    fn get_child_mut(&mut self, handle: &UIElementHandle) -> Option<&mut Box<dyn UIElement>> {
        for p in &mut self.pages {
            if p.get_handle() == handle {
                return Some(p);
            }
            if let Some(c) = p.get_child_mut(handle) {
                return Some(c);
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
        for t in &mut self.tabs {
            t.bg.set_z_index(z_index + 1.0);
            t.text.set_z_index(z_index + 2.0);
        }
        for p in &mut self.pages {
            p.set_z_index(z_index + 1.0);
        }
    }
}
