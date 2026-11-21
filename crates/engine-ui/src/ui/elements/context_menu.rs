//! A menu of actions that opens at a point (a right click, below a
//! button) and closes on a choice or a click elsewhere. Owned and driven by
//! the element that opens it: it is not a `UIElement` of its own.

use ferrite::core::{
    primitives::{Position, Region, Size},
    renderer::{
        plane::{Plane, PlaneBuilder, PlaneRenderer},
        text::Text,
    },
};
use glfw::{Action, MouseButton, Window, WindowEvent};

use crate::ui::theme;

/// One line of a `ContextMenu`.
#[derive(Clone, Debug, PartialEq)]
pub enum ContextEntry {
    /// An action. `danger` draws it in the warning colour (Delete).
    Item { icon: Option<&'static str>, label: String, enabled: bool, danger: bool },
    /// A thin line between groups.
    Separator,
}

impl ContextEntry {
    pub fn item(icon: &'static str, label: impl Into<String>) -> Self {
        ContextEntry::Item { icon: Some(icon), label: label.into(), enabled: true, danger: false }
    }

    pub fn danger(icon: &'static str, label: impl Into<String>) -> Self {
        ContextEntry::Item { icon: Some(icon), label: label.into(), enabled: true, danger: true }
    }

    pub fn enabled(mut self, on: bool) -> Self {
        if let ContextEntry::Item { enabled, .. } = &mut self {
            *enabled = on;
        }
        self
    }
}

struct Row {
    entry: ContextEntry,
    icon: Option<Text>,
    label: Option<Text>,
    bg: Plane,
    region: Region,
}

/// What a press did to an open menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextResult {
    /// The item at this index (counting separators) was chosen.
    Chosen(usize),
    /// The menu closed without a choice; the press landed outside it.
    Dismissed,
    /// The press landed on the menu but not on an item.
    Inside,
}

pub struct ContextMenu {
    rows: Vec<Row>,
    frame: Plane,
    at: (f32, f32),
    width: f32,
    open: bool,
    hover: Option<usize>,
}

const Z: f32 = 60.0;
const PADDING: f32 = 4.0;
const ROW: f32 = 28.0;
const SEPARATOR: f32 = 9.0;

impl Default for ContextMenu {
    fn default() -> Self {
        Self::new()
    }
}

impl ContextMenu {
    pub fn new() -> Self {
        Self {
            rows: Vec::new(),
            frame: PlaneBuilder::new()
                .color(theme::BG_MENU)
                .border_color(theme::BORDER_STRONG)
                .border_thickness(1.0)
                .border_radius_uniform(theme::RADIUS)
                .build(),
            at: (0.0, 0.0),
            width: 0.0,
            open: false,
            hover: None,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn close(&mut self) {
        self.open = false;
        self.hover = None;
    }

    /// Show `entries` with the menu's top left corner at `(x, y)` (moved in
    /// where it would leave the window).
    pub fn open_at(&mut self, x: f32, y: f32, entries: Vec<ContextEntry>) {
        self.rows = entries
            .into_iter()
            .map(|entry| {
                let (icon, label) = match &entry {
                    ContextEntry::Item { icon, label, enabled, danger } => {
                        let color = match (enabled, danger) {
                            (false, _) => theme::TEXT_DIM,
                            (true, true) => theme::DANGER,
                            (true, false) => theme::TEXT,
                        };
                        (
                            icon.map(|i| theme::icon(i, theme::ICON_SIZE, if *danger { theme::DANGER } else { theme::TEXT_MUTED })),
                            Some(theme::text(label, theme::FONT_SIZE, color)),
                        )
                    }
                    ContextEntry::Separator => (None, None),
                };
                Row {
                    entry,
                    icon,
                    label,
                    bg: PlaneBuilder::new().color(theme::TRANSPARENT).border_radius_uniform(theme::RADIUS_SMALL).build(),
                    region: Region::default(),
                }
            })
            .collect();
        let widest = self.rows.iter().filter_map(|r| r.label.as_ref()).map(|l| l.measure_width()).fold(0.0f32, f32::max);
        self.width = (widest + 26.0 + 28.0).max(170.0).ceil();
        let height = self.height();
        let (window_w, window_h) = PlaneRenderer::get_size();
        let x = x.min(window_w - self.width - 4.0).max(0.0);
        let y = if y + height > window_h { (y - height).max(0.0) } else { y };
        self.at = (x, y);
        self.open = true;
        self.hover = None;
        self.layout();
    }

    fn height(&self) -> f32 {
        self.rows
            .iter()
            .map(|r| if r.entry == ContextEntry::Separator { SEPARATOR } else { ROW })
            .sum::<f32>()
            + PADDING * 2.0
    }

    fn layout(&mut self) {
        let (x, top) = self.at;
        self.frame.set_position(Position::new(x, top, Z));
        self.frame.set_size(Size::new(self.width, self.height()));
        let width = self.width - PADDING * 2.0;
        let mut y = top + PADDING;
        for (i, row) in self.rows.iter_mut().enumerate() {
            let h = if row.entry == ContextEntry::Separator { SEPARATOR } else { ROW };
            let p = Position::new(x + PADDING, y, Z + 1.0);
            row.region = Region::new(p, Size::new(width, h));
            if row.entry == ContextEntry::Separator {
                row.bg.set_position(Position::new(p.x + 6.0, p.y + h / 2.0, Z + 1.0));
                row.bg.set_size(Size::new(width - 12.0, 1.0));
                row.bg.set_color(theme::BORDER);
            } else {
                let enabled = matches!(row.entry, ContextEntry::Item { enabled: true, .. });
                row.bg.set_position(p);
                row.bg.set_size(row.region.size);
                row.bg.set_color(if enabled && self.hover == Some(i) { theme::ACCENT_SOFT } else { theme::TRANSPARENT });
                if let Some(icon) = &mut row.icon {
                    let iy = theme::center_y(icon, h);
                    icon.prepare_render_at(Position::new(p.x + 10.0, p.y + iy, Z + 2.0));
                    icon.set_z_index(Z + 2.0);
                }
                if let Some(label) = &mut row.label {
                    let ly = theme::center_y(label, h);
                    label.prepare_render_at(Position::new(p.x + 36.0, p.y + ly, Z + 2.0));
                    label.set_z_index(Z + 2.0);
                }
            }
            y += h;
        }
    }

    /// Draw the menu (in the overlay pass, above the panels).
    pub fn render(&self) {
        if !self.open {
            return;
        }
        self.frame.render();
        for row in &self.rows {
            row.bg.render();
            if let Some(icon) = &row.icon {
                icon.render();
            }
            if let Some(label) = &row.label {
                label.render();
            }
        }
    }

    /// Follow the cursor and take presses while open. `None`: the event
    /// was not for the menu (or it is closed).
    pub fn handle_event(&mut self, window: &Window, event: &WindowEvent) -> Option<ContextResult> {
        if !self.open {
            return None;
        }
        match event {
            WindowEvent::CursorPos(x, y) => {
                let (x, y) = (*x as f32, *y as f32);
                let hover = self.rows.iter().position(|r| r.region.contains(x, y));
                if hover != self.hover {
                    self.hover = hover;
                    self.layout();
                }
                self.frame.get_region().contains(x, y).then_some(ContextResult::Inside)
            }
            WindowEvent::MouseButton(_, Action::Press, _) => {
                let (x, y) = window.get_cursor_pos();
                let (x, y) = (x as f32, y as f32);
                if !self.frame.get_region().contains(x, y) {
                    self.close();
                    return Some(ContextResult::Dismissed);
                }
                Some(ContextResult::Inside)
            }
            WindowEvent::MouseButton(MouseButton::Button1, Action::Release, _) => {
                let (x, y) = window.get_cursor_pos();
                let (x, y) = (x as f32, y as f32);
                let hit = self.rows.iter().position(|r| r.region.contains(x, y));
                match hit.map(|i| (i, &self.rows[i].entry)) {
                    Some((i, ContextEntry::Item { enabled: true, .. })) => {
                        self.close();
                        Some(ContextResult::Chosen(i))
                    }
                    _ => self.frame.get_region().contains(x, y).then_some(ContextResult::Inside),
                }
            }
            WindowEvent::Key(glfw::Key::Escape, _, Action::Press, _) => {
                self.close();
                Some(ContextResult::Dismissed)
            }
            _ => None,
        }
    }
}
