use ferrite::core::{
    primitives::{Offset, Position, Region, Size},
    renderer::{
        plane::{Plane, PlaneBuilder},
        text::Text,
    },
    scene::Scene,
};
use glfw::{
    Action::Press,
    Glfw, MouseButtonLeft, Window,
    WindowEvent::{self, CursorPos, MouseButton},
};

use crate::ui::{element_handle::UIElementHandle, theme, UIElement};

use super::button::{Button, ButtonStyle};

/// One entry of a `MenuButton`.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuItem {
    pub icon: Option<&'static str>,
    pub label: String,
    /// Shown right-aligned and dimmed, e.g. "Ctrl+S".
    pub shortcut: Option<String>,
}

impl MenuItem {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            icon: None,
            label: label.into(),
            shortcut: None,
        }
    }

    pub fn icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }
}

struct Entry {
    item: MenuItem,
    icon: Option<Text>,
    label: Text,
    shortcut: Option<Text>,
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
const MENU_PADDING: f32 = 4.0;

impl MenuButton {
    /// Plain text items.
    pub fn new(label: &str, items: Vec<String>, on_select: Box<dyn Fn(usize, &mut Scene)>) -> Self {
        Self::with_items(label, None, items.into_iter().map(MenuItem::new).collect(), on_select)
    }

    pub fn with_items(
        label: &str,
        icon: Option<&str>,
        items: Vec<MenuItem>,
        on_select: Box<dyn Fn(usize, &mut Scene)>,
    ) -> Self {
        let mut button = Button::new(Position::default(), Size::new(10.0, 26.0), Box::new(|_| {}));
        button.set_style(ButtonStyle::Ghost);
        if let Some(icon) = icon {
            button.set_icon(icon);
        }
        if !label.is_empty() {
            button.set_label(label);
        }
        button.fit(26.0);
        let size = *button.get_size();
        let mut menu_button = Self {
            handle: UIElementHandle::new(),
            region: Region::new(Position::default(), size),
            button,
            entries: Vec::new(),
            on_select,
            pending: None,
            is_open: false,
            hover: None,
            row_height: 28.0,
            menu_width: 0.0,
            menu: PlaneBuilder::new()
                .color(theme::BG_MENU)
                .border_color(theme::BORDER_STRONG)
                .border_thickness(1.0)
                .border_radius_uniform(theme::RADIUS)
                .build(),
        };
        menu_button.set_menu_items(items);
        menu_button
    }

    /// Use another trigger look than the default ghost button.
    pub fn set_button_style(&mut self, style: ButtonStyle) {
        self.button.set_style(style);
    }

    pub fn set_items(&mut self, items: Vec<String>) {
        self.set_menu_items(items.into_iter().map(MenuItem::new).collect());
    }

    pub fn set_menu_items(&mut self, items: Vec<MenuItem>) {
        if self.entries.iter().map(|e| &e.item).eq(items.iter()) {
            return;
        }
        self.entries = items
            .into_iter()
            .map(|item| Entry {
                icon: item
                    .icon
                    .map(|i| theme::icon(i, theme::ICON_SIZE, theme::TEXT_MUTED)),
                label: theme::text(&item.label, theme::FONT_SIZE, theme::TEXT),
                shortcut: item
                    .shortcut
                    .as_deref()
                    .map(|s| theme::text(s, theme::FONT_SIZE_SMALL, theme::TEXT_DIM)),
                item,
                bg: PlaneBuilder::new()
                    .color(theme::TRANSPARENT)
                    .border_radius_uniform(theme::RADIUS_SMALL)
                    .build(),
                region: Region::default(),
            })
            .collect();
        let has_icons = self.entries.iter().any(|e| e.icon.is_some());
        let widest = self
            .entries
            .iter()
            .map(|e| {
                e.label.measure_width()
                    + e.shortcut.as_ref().map_or(0.0, |s| s.measure_width() + 32.0)
            })
            .fold(0.0f32, f32::max);
        let icon_column = if has_icons { 26.0 } else { 0.0 };
        self.menu_width = (widest + icon_column + 28.0)
            .max(self.region.size.width)
            .max(160.0)
            .ceil();
    }

    fn layout_menu(&mut self) {
        let base = self.region.get_absolute_position();
        let top = base.y + self.region.size.height + 4.0;
        let has_icons = self.entries.iter().any(|e| e.icon.is_some());
        self.menu.set_position(Position::new(base.x, top, OVERLAY_Z));
        self.menu.set_size(Size::new(
            self.menu_width,
            self.entries.len() as f32 * self.row_height + MENU_PADDING * 2.0,
        ));
        for (i, entry) in self.entries.iter_mut().enumerate() {
            let p = Position::new(
                base.x + MENU_PADDING,
                top + MENU_PADDING + i as f32 * self.row_height,
                OVERLAY_Z + 1.0,
            );
            let width = self.menu_width - MENU_PADDING * 2.0;
            entry.region = Region::new(p, Size::new(width, self.row_height));
            entry.bg.set_position(p);
            entry.bg.set_size(entry.region.size);
            let hovered = self.hover == Some(i);
            entry.bg.set_color(if hovered {
                theme::ACCENT_SOFT
            } else {
                theme::TRANSPARENT
            });
            let mut x = p.x + 10.0;
            if let Some(icon) = &mut entry.icon {
                icon.set_color(if hovered { theme::ACCENT_TEXT } else { theme::TEXT_MUTED });
                let y = theme::center_y(icon, self.row_height);
                icon.prepare_render_at(Position::new(x, p.y + y, OVERLAY_Z + 2.0));
                icon.set_z_index(OVERLAY_Z + 2.0);
            }
            if has_icons {
                x += 26.0;
            }
            let y = theme::center_y(&entry.label, self.row_height);
            entry
                .label
                .prepare_render_at(Position::new(x, p.y + y, OVERLAY_Z + 2.0));
            entry.label.set_z_index(OVERLAY_Z + 2.0);
            if let Some(shortcut) = &mut entry.shortcut {
                let sx = p.x + width - 10.0 - shortcut.measure_width();
                let y = theme::center_y(shortcut, self.row_height);
                shortcut.prepare_render_at(Position::new(sx, p.y + y, OVERLAY_Z + 2.0));
                shortcut.set_z_index(OVERLAY_Z + 2.0);
            }
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
                if let Some(icon) = &entry.icon {
                    icon.render();
                }
                entry.label.render();
                if let Some(shortcut) = &entry.shortcut {
                    shortcut.render();
                }
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
