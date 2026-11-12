use ferrite::core::{
    primitives::{Offset, Position, Region, Size},
    renderer::{
        plane::{Plane, PlaneBuilder},
        text::{Fonts, Text as FerriteText},
    },
    scene::Scene,
};
use glfw::{
    Action::Press,
    Glfw, MouseButtonLeft, Window,
    WindowEvent::{self, CursorPos, MouseButton},
};

use crate::ui::{element_handle::UIElementHandle, UIElement};

use super::button::Button;

struct Entry {
    label: String,
    text: FerriteText,
    bg: Plane,
    region: Region,
}

/// A button that opens a list of actions. Choosing one calls `on_select`
/// with its index; the menu closes on selection or on a click elsewhere.
pub struct MenuButton {
    handle: UIElementHandle,
    region: Region,
    button: Button,
    entries: Vec<Entry>,
    on_select: Box<dyn Fn(usize, &mut Scene)>,
    pending: Option<usize>,
    is_open: bool,
    hover: Option<usize>,
    row_height: f32,
    menu_width: f32,
    menu: Plane,
}

const OVERLAY_Z: f32 = 40.0;

impl MenuButton {
    pub fn new(
        label: &str,
        items: Vec<String>,
        on_select: Box<dyn Fn(usize, &mut Scene)>,
    ) -> Self {
        let width = (label.chars().count() as f32 * 9.0 + 16.0).max(24.0);
        let size = Size::new(width, 20.0);
        let mut button = Button::new(Position::default(), size, Box::new(|_| {}));
        button.set_label(label);
        let mut menu_button = Self {
            handle: UIElementHandle::new(),
            region: Region::new(Position::default(), size),
            button,
            entries: Vec::new(),
            on_select,
            pending: None,
            is_open: false,
            hover: None,
            row_height: 20.0,
            menu_width: 0.0,
            menu: PlaneBuilder::new()
                .color((0.15, 0.15, 0.15, 1.0))
                .border_color((0.6, 0.6, 0.6, 1.0))
                .border_thickness(1.0)
                .border_radius_uniform(3.0)
                .build(),
        };
        menu_button.set_items(items);
        menu_button
    }

    pub fn set_items(&mut self, items: Vec<String>) {
        let current: Vec<&String> = self.entries.iter().map(|e| &e.label).collect();
        if current == items.iter().collect::<Vec<_>>() {
            return;
        }
        self.entries = items
            .into_iter()
            .map(|label| Entry {
                text: FerriteText::new(Fonts::RobotoMono, 0, 0, 0, 14.0, label.clone()),
                label,
                bg: PlaneBuilder::new().color((0.0, 0.0, 0.0, 0.0)).build(),
                region: Region::default(),
            })
            .collect();
        let widest = self
            .entries
            .iter()
            .map(|e| e.text.measure_width())
            .fold(0.0f32, f32::max);
        self.menu_width = (widest + 20.0).max(self.region.size.width).max(100.0);
    }

    fn layout_menu(&mut self) {
        let base = self.region.get_absolute_position();
        let top = base.y + self.region.size.height;
        self.menu.set_position(Position::new(base.x, top, OVERLAY_Z));
        self.menu.set_size(Size::new(
            self.menu_width,
            self.entries.len() as f32 * self.row_height + 4.0,
        ));
        for (i, entry) in self.entries.iter_mut().enumerate() {
            let p = Position::new(
                base.x + 2.0,
                top + 2.0 + i as f32 * self.row_height,
                OVERLAY_Z + 1.0,
            );
            entry.region = Region::new(p, Size::new(self.menu_width - 4.0, self.row_height));
            entry.bg.set_position(p);
            entry.bg.set_size(entry.region.size);
            entry.bg.set_color(if self.hover == Some(i) {
                (0.25, 0.4, 0.6, 1.0)
            } else {
                (0.0, 0.0, 0.0, 0.0)
            });
            entry.text.prepare_render_at(&p + (6.0, 2.0, 1.0));
            entry.text.set_z_index(OVERLAY_Z + 2.0);
        }
    }
}

impl UIElement for MenuButton {
    fn update(&mut self, scene: &mut Scene) {
        if let Some(index) = self.pending.take() {
            (self.on_select)(index, scene);
        }
        self.button.update(scene);
        if self.is_open {
            self.layout_menu();
        }
    }

    fn render(&self) {
        self.button.render();
    }

    fn render_overlay(&self) {
        if self.is_open {
            self.menu.render();
            for entry in &self.entries {
                entry.bg.render();
                entry.text.render();
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
        match event {
            MouseButton(MouseButtonLeft, Press, _) => {
                let (x, y) = window.get_cursor_pos();
                let (x, y) = (x as f32, y as f32);
                if self.region.contains(x, y) {
                    self.is_open = !self.is_open;
                    self.button.set_active(self.is_open);
                    return true;
                }
                if self.is_open {
                    let hit = self.entries.iter().position(|e| e.region.contains(x, y));
                    self.is_open = false;
                    self.button.set_active(false);
                    if let Some(index) = hit {
                        // Run on the next update, where the callback gets the
                        // same `&mut Scene` access as everything else.
                        self.pending = Some(index);
                        return true;
                    }
                    // Swallow the click if it landed on the menu frame.
                    let menu = self.menu.get_region();
                    return menu.contains(x, y);
                }
                false
            }
            CursorPos(x, y) => {
                self.button.handle_events(scene, window, glfw, event);
                if self.is_open {
                    let (x, y) = (*x as f32, *y as f32);
                    self.hover = self.entries.iter().position(|e| e.region.contains(x, y));
                    return self.menu.get_region().contains(x, y);
                }
                false
            }
            _ => false,
        }
    }

    leaf_children_methods!();

    fn get_handle(&self) -> &UIElementHandle {
        &self.handle
    }
    fn get_offset(&self) -> &Offset {
        &self.region.offset
    }
    fn set_offset(&mut self, offset: Offset) {
        self.region.offset = offset;
        self.button.set_offset(offset);
    }
    fn get_size(&self) -> &Size {
        &self.region.size
    }
    fn set_z_index(&mut self, z_index: f32) {
        self.region.position.z = z_index;
        self.button.set_z_index(z_index);
    }
}
