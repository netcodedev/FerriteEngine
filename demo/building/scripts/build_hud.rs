//! The builder's 2D overlay: the crosshair, the inventory at the bottom,
//! what the crosshair is on, messages, and the radial menu of blueprints.
//! Drawn with the engine's planes and text, in the viewport's pixels.

use std::cell::RefCell;
use std::f32::consts::{FRAC_PI_2, TAU};

use ferrite::core::{
    primitives::{Position, Size},
    renderer::{
        plane::{PlaneBuilder, PlaneRenderer},
        text::{Fonts, Text},
    },
};

use crate::building_parts::{Material, PieceKind};

pub type Color = (f32, f32, f32, f32);

const PANEL: Color = (0.07, 0.08, 0.09, 0.72);
const PANEL_BORDER: Color = (1.0, 1.0, 1.0, 0.10);
const ACCENT: Color = (0.98, 0.76, 0.36, 1.0);
const TEXT: Color = (0.94, 0.94, 0.92, 1.0);
const TEXT_DIM: Color = (0.70, 0.71, 0.72, 1.0);
const BLUEPRINT: Color = (0.30, 0.62, 0.98, 1.0);
const STONE: Color = (0.60, 0.58, 0.55, 1.0);
const STONE_DARK: Color = (0.46, 0.45, 0.43, 1.0);
const WOOD: Color = (0.58, 0.39, 0.22, 1.0);
const WOOD_DARK: Color = (0.42, 0.27, 0.15, 1.0);

/// What sits in the inventory's slots, left to right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Blueprint,
    Stones,
    Beams,
}

impl Item {
    pub const ALL: [Item; 3] = [Item::Blueprint, Item::Stones, Item::Beams];

    pub fn name(self) -> &'static str {
        match self {
            Item::Blueprint => "Bauplan",
            Item::Stones => "Steine",
            Item::Beams => "Balken",
        }
    }

    /// The material it puts into blueprints.
    pub fn material(self) -> Option<Material> {
        match self {
            Item::Blueprint => None,
            Item::Stones => Some(Material::Stone),
            Item::Beams => Some(Material::Beam),
        }
    }
}

/// Which option of the radial menu lies toward `cursor` (offset from the
/// menu's centre, y down), if it is far enough from the centre. The
/// options go clockwise from the top.
pub fn radial_choice(cursor: (f32, f32), options: usize, dead_zone: f32) -> Option<usize> {
    let (x, y) = cursor;
    if x * x + y * y < dead_zone * dead_zone || options == 0 {
        return None;
    }
    // Clockwise from straight up.
    let angle = (x.atan2(-y) + TAU) % TAU;
    let step = TAU / options as f32;
    Some(((angle + step / 2.0) / step) as usize % options)
}

/// Where option `i` of `options` sits, as an offset from the centre at
/// `radius` (y down).
fn radial_offset(i: usize, options: usize, radius: f32) -> (f32, f32) {
    let angle = i as f32 / options as f32 * TAU - FRAC_PI_2;
    (angle.cos() * radius, angle.sin() * radius)
}

/// What the HUD shows this frame.
pub struct HudState<'a> {
    pub selected: Item,
    pub blueprint: PieceKind,
    /// The radial menu, when open: the cursor's offset from its centre.
    pub radial: Option<(f32, f32)>,
    /// What the crosshair is on: name, parts in, parts in all.
    pub target: Option<(&'a str, usize, usize)>,
    /// The line of help above the inventory.
    pub hint: &'a str,
    /// A message and how visible it still is (0 to 1).
    pub message: Option<(&'a str, f32, bool)>,
}

/// Draws the HUD. Keeps its texts from frame to frame (laying out a text
/// costs), one per line drawn, in drawing order.
#[derive(Default)]
pub struct Hud {
    texts: RefCell<Vec<Text>>,
    used: RefCell<usize>,
}

/// Where text goes relative to its point.
#[derive(Clone, Copy)]
enum Align {
    Left,
    Center,
}

impl Hud {
    fn rect(&self, x: f32, y: f32, w: f32, h: f32, color: Color, radius: f32, border: Option<(f32, Color)>) {
        let mut builder = PlaneBuilder::new()
            .position(Position { x, y, z: 0.0 })
            .size(Size { width: w, height: h })
            .color(color)
            .border_radius_uniform(radius);
        if let Some((thickness, color)) = border {
            builder = builder.border_thickness(thickness).border_color(color);
        }
        PlaneRenderer::render(&builder.build());
    }

    fn circle(&self, cx: f32, cy: f32, r: f32, color: Color, border: Option<(f32, Color)>) {
        self.rect(cx - r, cy - r, r * 2.0, r * 2.0, color, r, border);
    }

    /// Text with its top at `y`; returns its width.
    fn text(&self, content: &str, x: f32, y: f32, size: f32, color: Color, font: Fonts, align: Align) -> f32 {
        let mut texts = self.texts.borrow_mut();
        let mut used = self.used.borrow_mut();
        if *used == texts.len() {
            texts.push(Text::new(font, 0, 0, 0, size, String::new()));
        }
        let text = &mut texts[*used];
        *used += 1;
        text.set_font(font, size);
        text.set_content(content);
        let width = text.measure_width();
        let left = match align {
            Align::Left => x,
            Align::Center => x - width / 2.0,
        };
        text.prepare_render_at(Position { x: left.round(), y: y.round(), z: 0.0 });
        text.set_color(color);
        text.render();
        width
    }

    pub fn draw(&self, state: &HudState) {
        *self.used.borrow_mut() = 0;
        let (w, h) = PlaneRenderer::get_size();
        let (cx, cy) = (w / 2.0, h / 2.0);

        if state.radial.is_none() {
            self.crosshair(cx, cy);
        }
        if let (Some((name, done, total)), None) = (state.target, state.radial) {
            self.target(cx, cy, name, done, total);
        }
        let bar_top = self.inventory(w, h, state.selected, state.blueprint);
        self.text(state.hint, cx, bar_top - 30.0, 15.0, TEXT_DIM, Fonts::Inter, Align::Center);
        if let Some((message, alpha, warning)) = state.message {
            let color = if warning { (1.0, 0.62, 0.45, alpha) } else { (0.62, 0.95, 0.62, alpha) };
            self.text(message, cx, bar_top - 62.0, 18.0, color, Fonts::InterSemiBold, Align::Center);
        }
        if let Some(cursor) = state.radial {
            self.radial(cx, cy, cursor, state.blueprint);
        }
    }

    fn crosshair(&self, cx: f32, cy: f32) {
        self.circle(cx, cy, 9.0, (0.0, 0.0, 0.0, 0.0), Some((1.5, (1.0, 1.0, 1.0, 0.55))));
        self.circle(cx, cy, 2.0, (1.0, 1.0, 1.0, 0.9), None);
    }

    /// The blueprint under the crosshair and how far it is built.
    fn target(&self, cx: f32, cy: f32, name: &str, done: usize, total: usize) {
        let (w, h) = (190.0, 46.0);
        let (x, y) = (cx + 28.0, cy - h / 2.0);
        self.rect(x, y, w, h, PANEL, 8.0, Some((1.0, PANEL_BORDER)));
        self.text(name, x + 12.0, y + 7.0, 15.0, TEXT, Fonts::InterSemiBold, Align::Left);
        let count = format!("{done}/{total}");
        self.text(&count, x + w - 44.0, y + 8.0, 14.0, TEXT_DIM, Fonts::Inter, Align::Left);
        let bar = w - 24.0;
        self.rect(x + 12.0, y + 31.0, bar, 6.0, (1.0, 1.0, 1.0, 0.12), 3.0, None);
        let fraction = if total == 0 { 0.0 } else { done as f32 / total as f32 };
        let fill = if done == total { (0.45, 0.85, 0.45, 1.0) } else { BLUEPRINT };
        if fraction > 0.0 {
            self.rect(x + 12.0, y + 31.0, (bar * fraction).max(6.0), 6.0, fill, 3.0, None);
        }
    }

    /// The inventory bar at the bottom; returns its top.
    fn inventory(&self, w: f32, h: f32, selected: Item, blueprint: PieceKind) -> f32 {
        let slot = 86.0;
        let gap = 10.0;
        let pad = 10.0;
        let count = Item::ALL.len() as f32;
        let bar_w = count * slot + (count - 1.0) * gap + 2.0 * pad;
        let bar_h = slot + 2.0 * pad;
        let x0 = (w - bar_w) / 2.0;
        let y0 = h - bar_h - 22.0;
        self.rect(x0, y0, bar_w, bar_h, PANEL, 14.0, Some((1.0, PANEL_BORDER)));
        for (i, item) in Item::ALL.into_iter().enumerate() {
            let x = x0 + pad + i as f32 * (slot + gap);
            let y = y0 + pad;
            let active = item == selected;
            let background = if active { (1.0, 1.0, 1.0, 0.13) } else { (1.0, 1.0, 1.0, 0.05) };
            let border = if active { (2.5, ACCENT) } else { (1.0, (1.0, 1.0, 1.0, 0.08)) };
            self.rect(x, y, slot, slot, background, 10.0, Some(border));
            self.item_icon(item, x + slot / 2.0, y + 34.0);
            let label = if item == Item::Blueprint && active { blueprint.name() } else { item.name() };
            let color = if active { TEXT } else { TEXT_DIM };
            self.text(label, x + slot / 2.0, y + slot - 24.0, 13.0, color, Fonts::Inter, Align::Center);
            self.text(&(i + 1).to_string(), x + 7.0, y + 4.0, 11.0, TEXT_DIM, Fonts::Inter, Align::Left);
        }
        y0
    }

    /// A small picture of an item, centred on `(cx, cy)`.
    fn item_icon(&self, item: Item, cx: f32, cy: f32) {
        match item {
            Item::Blueprint => {
                let (w, h) = (40.0, 30.0);
                let (x, y) = (cx - w / 2.0, cy - h / 2.0);
                self.rect(x, y, w, h, (0.16, 0.36, 0.66, 1.0), 3.0, Some((1.5, BLUEPRINT)));
                for i in 1..4 {
                    let gx = x + i as f32 * w / 4.0;
                    self.rect(gx, y + 3.0, 1.0, h - 6.0, (1.0, 1.0, 1.0, 0.25), 0.0, None);
                }
                for i in 1..3 {
                    let gy = y + i as f32 * h / 3.0;
                    self.rect(x + 3.0, gy, w - 6.0, 1.0, (1.0, 1.0, 1.0, 0.25), 0.0, None);
                }
                // A little house drawn on it.
                self.rect(cx - 8.0, cy - 2.0, 16.0, 10.0, (0.0, 0.0, 0.0, 0.0), 1.0, Some((1.5, (1.0, 1.0, 1.0, 0.9))));
                self.rect(cx - 2.0, cy + 2.0, 4.0, 6.0, (1.0, 1.0, 1.0, 0.9), 0.0, None);
                self.rect(cx - 10.0, cy - 6.0, 20.0, 3.0, (1.0, 1.0, 1.0, 0.9), 1.0, None);
            }
            Item::Stones => {
                let rows: [&[(f32, f32)]; 3] = [&[(-11.0, 20.0), (10.0, 18.0)], &[(-15.0, 13.0), (1.0, 18.0), (17.0, 10.0)], &[(-9.0, 24.0), (14.0, 18.0)]];
                for (r, row) in rows.iter().enumerate() {
                    for (k, (x, w)) in row.iter().enumerate() {
                        let color = if (r + k) % 2 == 0 { STONE } else { STONE_DARK };
                        self.rect(cx + x - w / 2.0, cy - 15.0 + r as f32 * 10.5, *w, 9.0, color, 3.0, None);
                    }
                }
            }
            Item::Beams => {
                for r in 0..3 {
                    let y = cy - 14.0 + r as f32 * 10.0;
                    let x = cx - 21.0 + (r % 2) as f32 * 3.0;
                    self.rect(x, y, 40.0, 8.0, WOOD, 2.0, None);
                    self.rect(x, y, 4.0, 8.0, WOOD_DARK, 2.0, None);
                    self.rect(x + 14.0, y + 3.5, 12.0, 1.0, WOOD_DARK, 0.0, None);
                }
            }
        }
    }

    /// A picture of a blueprint for the radial menu, centred on `(cx, cy)`.
    fn piece_icon(&self, kind: PieceKind, cx: f32, cy: f32, active: bool) {
        let line = if active { (1.0, 1.0, 1.0, 0.95) } else { (0.75, 0.86, 1.0, 0.85) };
        let fill = (0.30, 0.62, 0.98, if active { 0.45 } else { 0.25 });
        match kind {
            PieceKind::Foundation => {
                self.rect(cx - 26.0, cy - 4.0, 52.0, 8.0, fill, 2.0, Some((1.5, line)));
                for i in 0..3 {
                    self.rect(cx - 26.0 + i as f32 * 17.7, cy + 6.0, 16.0, 11.0, fill, 2.0, Some((1.5, line)));
                }
            }
            PieceKind::Wall | PieceKind::DoorWall => {
                let (w, h) = (44.0, 36.0);
                let (x, y) = (cx - w / 2.0, cy - h / 2.0);
                self.rect(x, y, w, h, fill, 2.0, Some((1.5, line)));
                for i in 1..6 {
                    self.rect(x + 2.0, y + i as f32 * h / 6.0, w - 4.0, 1.0, line, 0.0, None);
                }
                if kind == PieceKind::DoorWall {
                    self.rect(cx - 8.0, y + 12.0, 16.0, h - 12.0, (0.07, 0.08, 0.09, 1.0), 1.0, Some((1.5, line)));
                }
            }
        }
    }

    fn radial(&self, cx: f32, cy: f32, cursor: (f32, f32), current: PieceKind) {
        let (w, h) = PlaneRenderer::get_size();
        self.rect(0.0, 0.0, w, h, (0.0, 0.0, 0.0, 0.28), 0.0, None);
        let options = PieceKind::ALL.len();
        let radius = 118.0;
        let hovered = radial_choice(cursor, options, 34.0);
        self.circle(cx, cy, radius + 64.0, (0.06, 0.07, 0.08, 0.55), Some((1.0, PANEL_BORDER)));
        self.circle(cx, cy, 52.0, PANEL, Some((1.0, PANEL_BORDER)));
        for (i, kind) in PieceKind::ALL.into_iter().enumerate() {
            let (dx, dy) = radial_offset(i, options, radius);
            let (x, y) = (cx + dx, cy + dy);
            let active = hovered == Some(i);
            let r = if active { 56.0 } else { 50.0 };
            let border = if active {
                (2.5, ACCENT)
            } else if kind == current {
                (2.0, BLUEPRINT)
            } else {
                (1.0, PANEL_BORDER)
            };
            let background = if active { (0.16, 0.18, 0.21, 0.96) } else { (0.10, 0.11, 0.13, 0.92) };
            self.circle(x, y, r, background, Some(border));
            self.piece_icon(kind, x, y - 9.0, active);
            let color = if active { TEXT } else { TEXT_DIM };
            self.text(kind.name(), x, y + 20.0, 13.0, color, Fonts::InterSemiBold, Align::Center);
        }
        let label = hovered.map_or(current.name(), |i| PieceKind::ALL[i].name());
        self.text("Bauplan", cx, cy - 20.0, 12.0, TEXT_DIM, Fonts::Inter, Align::Center);
        self.text(label, cx, cy - 2.0, 16.0, TEXT, Fonts::InterSemiBold, Align::Center);
        // Where the mouse points.
        let (mx, my) = (cx + cursor.0, cy + cursor.1);
        self.circle(mx, my, 5.0, ACCENT, Some((1.5, (0.0, 0.0, 0.0, 0.6))));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_radial_menu_picks_the_option_the_mouse_points_at() {
        // Three options: top, lower right, lower left.
        assert_eq!(radial_choice((0.0, -100.0), 3, 30.0), Some(0));
        assert_eq!(radial_choice((90.0, 50.0), 3, 30.0), Some(1));
        assert_eq!(radial_choice((-90.0, 50.0), 3, 30.0), Some(2));
        // Near the centre nothing is picked.
        assert_eq!(radial_choice((5.0, 5.0), 3, 30.0), None);
        for i in 0..3 {
            let (x, y) = radial_offset(i, 3, 100.0);
            assert_eq!(radial_choice((x, y), 3, 30.0), Some(i));
        }
    }
}
