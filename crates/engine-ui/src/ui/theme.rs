//! Shared look of all widgets: colours, radii, fonts and text helpers.
//! Dark, low-contrast surfaces with one blue accent, in the spirit of
//! modern editors.

use ferrite::core::renderer::text::{Fonts, Text};

pub type Color = (f32, f32, f32, f32);

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0)
}

const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color {
    (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a)
}

// Surfaces, darkest to lightest.
pub const BG_APP: Color = rgb(13, 14, 17);
pub const BG_PANEL: Color = rgb(22, 23, 27);
pub const BG_HEADER: Color = rgb(18, 19, 23);
pub const BG_FIELD: Color = rgb(14, 15, 18);
pub const BG_CONTROL: Color = rgb(36, 38, 44);
pub const BG_CONTROL_HOVER: Color = rgb(46, 49, 57);
pub const BG_ROW_HOVER: Color = rgba(255, 255, 255, 0.05);
pub const BG_MENU: Color = rgb(28, 30, 35);
pub const TRANSPARENT: Color = (0.0, 0.0, 0.0, 0.0);
pub const OVERLAY_DIM: Color = rgba(0, 0, 0, 0.55);

pub const BORDER: Color = rgb(44, 46, 53);
pub const BORDER_STRONG: Color = rgb(62, 65, 75);

pub const TEXT: Color = rgb(229, 231, 235);
pub const TEXT_MUTED: Color = rgb(156, 160, 171);
pub const TEXT_DIM: Color = rgb(104, 108, 120);

pub const ACCENT: Color = rgb(86, 132, 250);
pub const ACCENT_HOVER: Color = rgb(110, 151, 255);
pub const ACCENT_SOFT: Color = rgba(86, 132, 250, 0.22);
pub const ACCENT_TEXT: Color = rgb(170, 196, 255);
pub const DANGER: Color = rgb(225, 72, 72);
pub const SUCCESS: Color = rgb(64, 186, 120);
pub const WARNING: Color = rgb(232, 170, 60);

pub const RADIUS: f32 = 6.0;
pub const RADIUS_SMALL: f32 = 4.0;

pub const FONT_SIZE: f32 = 14.0;
pub const FONT_SIZE_SMALL: f32 = 12.5;
pub const ICON_SIZE: f32 = 16.0;

/// Regular UI text.
pub fn text(content: &str, size: f32, color: Color) -> Text {
    let mut text = Text::new(Fonts::Inter, 0, 0, 0, size, content.to_owned());
    text.set_color(color);
    text
}

/// Semibold text for headings and titles.
pub fn heading(content: &str, size: f32, color: Color) -> Text {
    let mut text = Text::new(Fonts::InterSemiBold, 0, 0, 0, size, content.to_owned());
    text.set_color(color);
    text
}

/// A Lucide icon (see `ferrite::core::renderer::text::icons`).
pub fn icon(glyph: &str, size: f32, color: Color) -> Text {
    let mut text = Text::new(Fonts::Lucide, 0, 0, 0, size, glyph.to_owned());
    text.set_color(color);
    text
}

/// Monospace text for numbers that should line up (stats, values).
pub fn mono(content: &str, size: f32, color: Color) -> Text {
    let mut text = Text::new(Fonts::RobotoMono, 0, 0, 0, size, content.to_owned());
    text.set_color(color);
    text
}

/// Y offset that vertically centres a line of `text` in a box of `height`.
pub fn center_y(text: &Text, height: f32) -> f32 {
    ((height - text.line_height()) * 0.5).round()
}
