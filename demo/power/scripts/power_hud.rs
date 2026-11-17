//! The power demo's 2D overlay: crosshair, the inventory at the bottom,
//! what the crosshair is on (a device, its network), the cable being
//! drawn (length, sag, Shift lock), the time and weather, messages.
//! Drawn with the engine's planes and text, in the viewport's pixels.

use std::cell::RefCell;

use ferrite::core::{
    primitives::{Position, Size},
    renderer::{
        plane::{PlaneBuilder, PlaneRenderer},
        text::{Fonts, Text},
    },
};

use crate::devices::DeviceKind;

pub type Color = (f32, f32, f32, f32);

pub const PANEL: Color = (0.07, 0.08, 0.09, 0.74);
pub const PANEL_BORDER: Color = (1.0, 1.0, 1.0, 0.10);
pub const ACCENT: Color = (0.98, 0.78, 0.30, 1.0);
pub const TEXT: Color = (0.94, 0.94, 0.92, 1.0);
pub const TEXT_DIM: Color = (0.68, 0.70, 0.72, 1.0);
pub const GOOD: Color = (0.50, 0.90, 0.50, 1.0);
pub const WARN: Color = (1.0, 0.62, 0.40, 1.0);
pub const POWER: Color = (1.0, 0.82, 0.30, 1.0);
pub const STORE: Color = (0.40, 0.78, 1.0, 1.0);

/// What sits in the inventory's slots, left to right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Cable,
    Device(DeviceKind),
}

impl Item {
    pub const ALL: [Item; 9] = [
        Item::Cable,
        Item::Device(DeviceKind::Solar),
        Item::Device(DeviceKind::Wind),
        Item::Device(DeviceKind::Generator),
        Item::Device(DeviceKind::Battery),
        Item::Device(DeviceKind::Lamp),
        Item::Device(DeviceKind::Floodlight),
        Item::Device(DeviceKind::Heater),
        Item::Device(DeviceKind::Pole),
    ];

    pub fn name(self) -> &'static str {
        match self {
            Item::Cable => "Kabel",
            Item::Device(kind) => kind.name(),
        }
    }

    pub fn index(self) -> usize {
        Item::ALL.iter().position(|i| *i == self).unwrap_or(0)
    }
}

/// A panel of lines next to the crosshair: what it is on.
pub struct Panel {
    pub title: String,
    pub lines: Vec<(String, Color)>,
    /// A bar under the lines: how full (0..1) and its colour.
    pub bar: Option<(f32, Color)>,
}

/// The cable being drawn.
pub struct CableInfo<'a> {
    pub length: f32,
    pub max: f32,
    pub sag: f32,
    pub lock: Option<&'a str>,
    pub points: usize,
    pub problem: Option<&'a str>,
    pub from: &'a str,
}

/// What the HUD shows this frame.
pub struct HudState<'a> {
    pub selected: Item,
    pub hint: &'a str,
    pub message: Option<(&'a str, f32, bool)>,
    pub target: Option<Panel>,
    pub network: Option<Panel>,
    pub cable: Option<CableInfo<'a>>,
    pub clock: (u32, u32),
    pub weather: &'a str,
    pub wind: f32,
    pub sun: f32,
    pub warp: bool,
}

/// Draws the HUD. Keeps its texts from frame to frame (laying out a text
/// costs), one per line drawn, in drawing order.
#[derive(Default)]
pub struct Hud {
    texts: RefCell<Vec<Text>>,
    used: RefCell<usize>,
}

#[derive(Clone, Copy)]
enum Align {
    Left,
    Center,
    Right,
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
            Align::Right => x - width,
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
        self.crosshair(cx, cy, state.selected == Item::Cable);
        if let Some(panel) = &state.target {
            self.panel(cx + 30.0, cy - 30.0, 250.0, panel);
        }
        if let Some(panel) = &state.network {
            self.panel(20.0, 20.0, 270.0, panel);
        }
        if let Some(cable) = &state.cable {
            self.cable(cx, cy, cable);
        }
        self.sky(w, state);
        let bar_top = self.inventory(w, h, state.selected);
        self.text(state.hint, cx, bar_top - 28.0, 15.0, TEXT_DIM, Fonts::Inter, Align::Center);
        if let Some((message, alpha, warning)) = state.message {
            let color = if warning { (1.0, 0.62, 0.45, alpha) } else { (0.62, 0.95, 0.62, alpha) };
            self.text(message, cx, bar_top - 60.0, 18.0, color, Fonts::InterSemiBold, Align::Center);
        }
    }

    fn crosshair(&self, cx: f32, cy: f32, cable: bool) {
        let ring = if cable { (1.0, 0.85, 0.35, 0.75) } else { (1.0, 1.0, 1.0, 0.55) };
        self.circle(cx, cy, 9.0, (0.0, 0.0, 0.0, 0.0), Some((1.5, ring)));
        self.circle(cx, cy, 2.0, (1.0, 1.0, 1.0, 0.9), None);
    }

    /// A panel of a title, lines and maybe a bar, its top left at `(x, y)`.
    fn panel(&self, x: f32, y: f32, w: f32, panel: &Panel) {
        let line_h = 19.0;
        let bar_h = if panel.bar.is_some() { 16.0 } else { 0.0 };
        let h = 34.0 + panel.lines.len() as f32 * line_h + bar_h + 6.0;
        self.rect(x, y, w, h, PANEL, 8.0, Some((1.0, PANEL_BORDER)));
        self.text(&panel.title, x + 12.0, y + 8.0, 15.0, TEXT, Fonts::InterSemiBold, Align::Left);
        for (i, (line, color)) in panel.lines.iter().enumerate() {
            self.text(line, x + 12.0, y + 32.0 + i as f32 * line_h, 13.0, *color, Fonts::Inter, Align::Left);
        }
        if let Some((fraction, color)) = panel.bar {
            let by = y + 34.0 + panel.lines.len() as f32 * line_h + 2.0;
            let bw = w - 24.0;
            self.rect(x + 12.0, by, bw, 6.0, (1.0, 1.0, 1.0, 0.12), 3.0, None);
            if fraction > 0.0 {
                self.rect(x + 12.0, by, (bw * fraction.clamp(0.0, 1.0)).max(6.0), 6.0, color, 3.0, None);
            }
        }
    }

    /// Length and sag of the cable being drawn, under the crosshair.
    fn cable(&self, cx: f32, cy: f32, cable: &CableInfo) {
        let (w, h) = (300.0, 92.0);
        let (x, y) = (cx - w / 2.0, cy + 34.0);
        let bad = cable.problem.is_some();
        let border = if bad { (1.5, WARN) } else { (1.0, PANEL_BORDER) };
        self.rect(x, y, w, h, PANEL, 8.0, Some(border));
        let title = format!("Kabel von {}", cable.from);
        self.text(&title, x + 12.0, y + 8.0, 14.0, TEXT, Fonts::InterSemiBold, Align::Left);
        let length = format!("{:.1} / {:.0} m", cable.length, cable.max);
        let over = cable.length > cable.max;
        self.text(&length, x + w - 12.0, y + 8.0, 14.0, if over { WARN } else { TEXT }, Fonts::InterSemiBold, Align::Right);
        let bw = w - 24.0;
        self.rect(x + 12.0, y + 30.0, bw, 5.0, (1.0, 1.0, 1.0, 0.12), 2.5, None);
        let fraction = (cable.length / cable.max).clamp(0.0, 1.0);
        let fill = if over { WARN } else if fraction > 0.8 { ACCENT } else { STORE };
        if fraction > 0.0 {
            self.rect(x + 12.0, y + 30.0, (bw * fraction).max(5.0), 5.0, fill, 2.5, None);
        }
        let sag = match cable.lock {
            Some(lock) => format!("Durchhang {:.2} m  ·  {} (Shift)", cable.sag, lock),
            None => format!("Durchhang {:.2} m  (Mausrad)", cable.sag),
        };
        self.text(&sag, x + 12.0, y + 44.0, 13.0, TEXT_DIM, Fonts::Inter, Align::Left);
        let points = format!("{} Halterungen", cable.points);
        self.text(&points, x + w - 12.0, y + 44.0, 13.0, TEXT_DIM, Fonts::Inter, Align::Right);
        if let Some(problem) = cable.problem {
            self.text(problem, x + 12.0, y + 66.0, 13.0, WARN, Fonts::InterSemiBold, Align::Left);
        } else {
            self.text("Klick: befestigen  ·  Rechtsklick: zurück", x + 12.0, y + 66.0, 13.0, TEXT_DIM, Fonts::Inter, Align::Left);
        }
    }

    /// Time, weather and wind, top right.
    fn sky(&self, w: f32, state: &HudState) {
        let (pw, ph) = (210.0, 78.0);
        let (x, y) = (w - pw - 20.0, 20.0);
        self.rect(x, y, pw, ph, PANEL, 8.0, Some((1.0, PANEL_BORDER)));
        let clock = format!("{:02}:{:02}", state.clock.0, state.clock.1);
        self.text(&clock, x + 12.0, y + 8.0, 18.0, TEXT, Fonts::InterSemiBold, Align::Left);
        if state.warp {
            self.text("Zeitraffer", x + pw - 12.0, y + 11.0, 12.0, ACCENT, Fonts::InterSemiBold, Align::Right);
        }
        self.text(state.weather, x + 12.0, y + 34.0, 13.0, TEXT_DIM, Fonts::Inter, Align::Left);
        let line = format!("Sonne {:.0} %  ·  Wind {:.1} m/s", state.sun * 100.0, state.wind);
        self.text(&line, x + 12.0, y + 53.0, 13.0, TEXT_DIM, Fonts::Inter, Align::Left);
    }

    /// The inventory bar at the bottom; returns its top.
    fn inventory(&self, w: f32, h: f32, selected: Item) -> f32 {
        let slot = 76.0;
        let gap = 8.0;
        let pad = 9.0;
        let count = Item::ALL.len() as f32;
        let bar_w = count * slot + (count - 1.0) * gap + 2.0 * pad;
        let bar_h = slot + 2.0 * pad;
        let x0 = (w - bar_w) / 2.0;
        let y0 = h - bar_h - 20.0;
        self.rect(x0, y0, bar_w, bar_h, PANEL, 14.0, Some((1.0, PANEL_BORDER)));
        for (i, item) in Item::ALL.into_iter().enumerate() {
            let x = x0 + pad + i as f32 * (slot + gap);
            let y = y0 + pad;
            let active = item == selected;
            let background = if active { (1.0, 1.0, 1.0, 0.13) } else { (1.0, 1.0, 1.0, 0.05) };
            let border = if active { (2.5, ACCENT) } else { (1.0, (1.0, 1.0, 1.0, 0.08)) };
            self.rect(x, y, slot, slot, background, 10.0, Some(border));
            self.icon(item, x + slot / 2.0, y + 30.0);
            let color = if active { TEXT } else { TEXT_DIM };
            self.text(item.name(), x + slot / 2.0, y + slot - 21.0, 11.5, color, Fonts::Inter, Align::Center);
            self.text(&(i + 1).to_string(), x + 6.0, y + 3.0, 10.0, TEXT_DIM, Fonts::Inter, Align::Left);
        }
        y0
    }

    /// A small picture of an item, centred on `(cx, cy)`.
    fn icon(&self, item: Item, cx: f32, cy: f32) {
        let none = (0.0, 0.0, 0.0, 0.0);
        match item {
            Item::Cable => {
                // A reel of cable.
                self.circle(cx, cy, 17.0, none, Some((3.0, (0.20, 0.20, 0.22, 1.0))));
                self.circle(cx, cy, 12.0, none, Some((3.0, (0.12, 0.12, 0.13, 1.0))));
                self.circle(cx, cy, 5.0, (0.55, 0.40, 0.25, 1.0), None);
                self.rect(cx + 14.0, cy + 8.0, 12.0, 3.0, (0.12, 0.12, 0.13, 1.0), 1.5, None);
                self.rect(cx + 24.0, cy + 6.0, 5.0, 7.0, (0.72, 0.42, 0.22, 1.0), 1.0, None);
            }
            Item::Device(kind) => match kind {
                DeviceKind::Solar => {
                    self.rect(cx - 22.0, cy - 13.0, 44.0, 24.0, (0.10, 0.16, 0.34, 1.0), 2.0, Some((1.5, (0.75, 0.77, 0.80, 1.0))));
                    for i in 1..4 {
                        self.rect(cx - 22.0 + i as f32 * 11.0, cy - 12.0, 1.0, 22.0, (0.55, 0.62, 0.78, 0.8), 0.0, None);
                    }
                    self.rect(cx - 21.0, cy - 1.5, 42.0, 1.0, (0.55, 0.62, 0.78, 0.8), 0.0, None);
                    self.rect(cx - 15.0, cy + 11.0, 2.0, 7.0, (0.70, 0.72, 0.75, 1.0), 0.0, None);
                    self.rect(cx + 13.0, cy + 11.0, 2.0, 7.0, (0.70, 0.72, 0.75, 1.0), 0.0, None);
                }
                DeviceKind::Wind => {
                    let white = (0.92, 0.92, 0.90, 1.0);
                    self.rect(cx - 1.5, cy - 6.0, 3.0, 26.0, white, 1.0, None);
                    self.rect(cx - 2.0, cy - 24.0, 4.0, 18.0, white, 2.0, None);
                    for k in 0..6 {
                        let t = 3.0 + k as f32 * 3.0;
                        self.circle(cx - t * 0.87, cy - 6.0 + t * 0.5, 2.0, white, None);
                        self.circle(cx + t * 0.87, cy - 6.0 + t * 0.5, 2.0, white, None);
                    }
                    self.circle(cx, cy - 6.0, 3.5, (0.6, 0.6, 0.6, 1.0), None);
                }
                DeviceKind::Generator => {
                    self.rect(cx - 22.0, cy - 12.0, 44.0, 26.0, (0.88, 0.46, 0.10, 1.0), 3.0, None);
                    self.rect(cx - 24.0, cy + 12.0, 48.0, 4.0, (0.15, 0.15, 0.16, 1.0), 1.0, None);
                    self.rect(cx + 2.0, cy - 6.0, 14.0, 10.0, (0.15, 0.15, 0.16, 1.0), 1.0, None);
                    self.circle(cx + 12.0, cy - 2.0, 2.0, GOOD, None);
                    self.rect(cx - 16.0, cy - 20.0, 4.0, 9.0, (0.15, 0.15, 0.16, 1.0), 1.0, None);
                }
                DeviceKind::Battery => {
                    self.rect(cx - 12.0, cy - 18.0, 24.0, 36.0, (0.82, 0.84, 0.86, 1.0), 3.0, None);
                    self.rect(cx - 5.0, cy - 22.0, 10.0, 4.0, (0.75, 0.15, 0.12, 1.0), 1.0, None);
                    for i in 0..4 {
                        let color = if i < 3 { (0.35, 0.85, 0.40, 1.0) } else { (0.2, 0.3, 0.22, 1.0) };
                        self.rect(cx - 7.0, cy + 10.0 - i as f32 * 8.0, 14.0, 5.0, color, 1.0, None);
                    }
                }
                DeviceKind::Lamp => {
                    self.circle(cx, cy - 6.0, 12.0, (1.0, 0.86, 0.55, 1.0), None);
                    self.circle(cx, cy - 6.0, 17.0, (1.0, 0.86, 0.55, 0.18), None);
                    self.rect(cx - 1.5, cy + 6.0, 3.0, 9.0, (0.15, 0.15, 0.16, 1.0), 0.0, None);
                    self.rect(cx - 9.0, cy + 14.0, 18.0, 4.0, (0.15, 0.15, 0.16, 1.0), 2.0, None);
                }
                DeviceKind::Floodlight => {
                    self.rect(cx - 16.0, cy - 18.0, 32.0, 22.0, (0.28, 0.29, 0.31, 1.0), 3.0, None);
                    self.rect(cx - 13.0, cy - 15.0, 26.0, 16.0, (1.0, 0.95, 0.80, 1.0), 2.0, None);
                    self.rect(cx - 1.5, cy + 4.0, 3.0, 12.0, (0.15, 0.15, 0.16, 1.0), 0.0, None);
                    self.rect(cx - 10.0, cy + 15.0, 20.0, 4.0, (0.15, 0.15, 0.16, 1.0), 1.0, None);
                }
                DeviceKind::Heater => {
                    self.rect(cx - 20.0, cy - 14.0, 40.0, 28.0, (0.90, 0.90, 0.88, 1.0), 4.0, None);
                    for i in 0..4 {
                        self.rect(cx - 15.0, cy - 9.0 + i as f32 * 6.0, 30.0, 2.5, (1.0, 0.42, 0.15, 1.0), 1.0, None);
                    }
                    self.rect(cx - 16.0, cy + 14.0, 6.0, 4.0, (0.15, 0.15, 0.16, 1.0), 1.0, None);
                    self.rect(cx + 10.0, cy + 14.0, 6.0, 4.0, (0.15, 0.15, 0.16, 1.0), 1.0, None);
                }
                DeviceKind::Pole => {
                    let wood = (0.45, 0.32, 0.20, 1.0);
                    self.rect(cx - 2.5, cy - 22.0, 5.0, 42.0, wood, 1.0, None);
                    self.rect(cx - 16.0, cy - 19.0, 32.0, 3.0, wood, 1.0, None);
                    self.circle(cx - 13.0, cy - 21.0, 2.0, (0.9, 0.92, 0.95, 1.0), None);
                    self.circle(cx + 13.0, cy - 21.0, 2.0, (0.9, 0.92, 0.95, 1.0), None);
                    self.rect(cx + 2.0, cy + 2.0, 8.0, 11.0, (0.62, 0.64, 0.62, 1.0), 1.0, None);
                    self.rect(cx + 4.5, cy + 4.0, 3.0, 3.0, (0.95, 0.75, 0.10, 1.0), 0.0, None);
                }
            },
        }
    }
}
