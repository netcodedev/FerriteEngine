use std::{str::FromStr, time::Instant};

use ferrite::core::{
    primitives::{Offset, Position, Region, Size},
    renderer::{
        plane::{PlaneBuilder, PlaneRenderer},
        text::{Fonts, Text},
    },
    scene::Scene,
    utils::DataSource,
};
use glfw::{
    Action::{Press, Release, Repeat},
    Key, Modifiers,
    WindowEvent::{self, Char, CursorPos, MouseButton},
};

use crate::ui::{element_handle::UIElementHandle, theme, UIElement};

use super::{Input, TextEdit};

/// Horizontal text padding inside the field.
const PAD_X: f32 = 8.0;
/// Clicks closer together than this count as double/triple clicks.
const MULTI_CLICK_SECONDS: f32 = 0.35;
const ELLIPSIS: char = '\u{2026}';

impl<T: Clone + ToString + FromStr> Input<T> {
    pub fn new(
        position: Position,
        size: Size,
        content: T,
        data_source: Option<DataSource<T>>,
    ) -> Self {
        // Derive the font size from the input's height so taller inputs get
        // bigger text automatically, capped at the UI font size.
        let font_size = ((size.height - 6.0).max(10.0)).min(size.height.max(10.0));
        let cursor_h = (font_size - 2.0).max(6.0);
        let plane = PlaneBuilder::new()
            .position(position)
            .size(size)
            .border_radius_uniform(theme::RADIUS)
            .border_color(theme::BORDER)
            .color(theme::BG_FIELD)
            .border_thickness(1.0);
        Self {
            handle: UIElementHandle::new(),
            region: Region::new(position, size),
            is_hovering: false,
            is_focused: false,
            dragging: false,
            last_click: None,
            scroll_offset: 0.0,
            blink_start: Instant::now(),
            edit: TextEdit::new(&content.to_string()),
            text: {
                let mut t = Text::new(
                    Fonts::Inter,
                    0,
                    0,
                    0,
                    font_size.min(theme::FONT_SIZE),
                    content.to_string(),
                );
                t.set_color(theme::TEXT);
                t
            },
            plane: plane.build(),
            stencil_plane: plane.size(&size - (2.0, 0.0)).build(),
            cursor_plane: PlaneBuilder::new()
                .position(position)
                .size(Size::new((font_size * 0.08).max(1.0), cursor_h))
                .color(theme::ACCENT)
                .build(),
            selection_plane: PlaneBuilder::new()
                .color(theme::ACCENT_SOFT)
                .border_radius_uniform(2.0)
                .build(),
            data_source,
        }
    }

    fn reset_blink(&mut self) {
        self.blink_start = Instant::now();
    }

    pub fn is_focused(&self) -> bool {
        self.is_focused
    }

    fn source_text(&self) -> Option<String> {
        self.data_source.as_ref().map(|s| s.to_string())
    }

    fn focus(&mut self) {
        if !self.is_focused {
            self.is_focused = true;
            // Editing always starts from the full, untruncated value.
            if let Some(text) = self.source_text() {
                self.edit.set_text(&text);
            }
        }
    }

    fn blur(&mut self) {
        self.is_focused = false;
        self.dragging = false;
        if let Some(text) = self.source_text() {
            self.edit.set_text(&text);
        }
        self.edit.set_cursor(0, false);
    }

    /// Write the edited text to the bound source when it parses.
    fn commit(&mut self) {
        if let Some(data_source) = &self.data_source {
            data_source.try_write_from_string(self.edit.text());
        }
        self.reset_blink();
    }

    pub fn set_handle(&mut self, handle: UIElementHandle) {
        self.handle = handle;
    }

    pub fn set_size(&mut self, size: Size) {
        self.region.size = size;
        self.plane.set_size(size);
        self.stencil_plane.set_size(Size::new(size.width - 2.0, size.height));
    }

    fn text_origin_x(&self) -> f32 {
        self.region.position.x + self.region.offset.x + PAD_X - self.scroll_offset
    }

    fn prefix_width(&self, chars: usize) -> f32 {
        let prefix: String = self.edit.text().chars().take(chars).collect();
        self.text.measure_str(&prefix)
    }

    /// Caret index closest to window x coordinate `x`.
    fn index_at(&self, x: f32) -> usize {
        let local = x - self.text_origin_x();
        let mut best = (0, local.abs());
        let mut prefix = String::new();
        for (i, c) in self.edit.text().chars().enumerate() {
            prefix.push(c);
            let d = (local - self.text.measure_str(&prefix)).abs();
            if d < best.1 {
                best = (i + 1, d);
            }
        }
        best.0
    }

    /// The text as shown without focus: cut with "…" if it does not fit.
    fn display_text(&self, full: &str) -> String {
        let visible = (self.region.size.width - PAD_X * 2.0).max(0.0);
        if self.text.measure_str(full) <= visible {
            return full.to_owned();
        }
        let chars: Vec<char> = full.chars().collect();
        for keep in (0..chars.len()).rev() {
            let candidate: String = chars[..keep]
                .iter()
                .chain(std::iter::once(&ELLIPSIS))
                .collect();
            if self.text.measure_str(&candidate) <= visible {
                return candidate;
            }
        }
        ELLIPSIS.to_string()
    }

    fn handle_key(&mut self, window: &mut glfw::Window, key: Key, mods: Modifiers) -> bool {
        let shift = mods.contains(Modifiers::Shift);
        let ctrl = mods.contains(Modifiers::Control) || mods.contains(Modifiers::Super);
        match key {
            Key::Left => self.edit.move_left(shift, ctrl),
            Key::Right => self.edit.move_right(shift, ctrl),
            Key::Home => self.edit.home(shift),
            Key::End => self.edit.end(shift),
            Key::Backspace => {
                self.edit.backspace(ctrl);
                self.commit();
            }
            Key::Delete => {
                self.edit.delete(ctrl);
                self.commit();
            }
            Key::A if ctrl => self.edit.select_all(),
            Key::C if ctrl => {
                if let Some(text) = self.edit.selected_text() {
                    window.set_clipboard_string(text);
                }
            }
            Key::X if ctrl => {
                if let Some(text) = self.edit.cut() {
                    window.set_clipboard_string(&text);
                    self.commit();
                }
            }
            Key::V if ctrl => {
                if let Some(text) = window.get_clipboard_string() {
                    // Single-line field: keep only the first line.
                    let line = text.lines().next().unwrap_or("").to_owned();
                    self.edit.insert(&line);
                    self.commit();
                }
            }
            // Other keys are swallowed so they do not trigger shortcuts.
            _ => return true,
        }
        self.reset_blink();
        true
    }
}

impl<T: Clone + ToString + FromStr> UIElement for Input<T> {
    fn update(&mut self, _scene: &mut Scene) {
        // While focused the typed text is authoritative: it may be a value
        // that does not parse yet ("-", "1."), so the bound source must not
        // replace it until editing ends.
        let base = &self.region.position + &self.region.offset;
        let visible_width = (self.region.size.width - PAD_X * 2.0).max(0.0);
        let shown = if self.is_focused {
            self.edit.text().to_owned()
        } else {
            let full = self
                .source_text()
                .unwrap_or_else(|| self.edit.text().to_owned());
            if full != self.edit.text() {
                self.edit.set_text(&full);
                self.edit.set_cursor(0, false);
            }
            self.display_text(&full)
        };
        self.text.set_content(&shown);
        let text_y = theme::center_y(&self.text, self.region.size.height);

        // Scroll just enough to keep the caret inside the visible area.
        if self.is_focused {
            let caret = self.prefix_width(self.edit.cursor());
            let text_width = self.text.measure_str(self.edit.text());
            if caret - self.scroll_offset > visible_width {
                self.scroll_offset = caret - visible_width;
            } else if caret < self.scroll_offset {
                self.scroll_offset = caret;
            }
            self.scroll_offset = self
                .scroll_offset
                .min((text_width - visible_width).max(0.0))
                .max(0.0);
        } else {
            self.scroll_offset = 0.0;
        }
        self.text
            .prepare_render_at(&base + (PAD_X - self.scroll_offset, text_y, 1.0));

        let origin = self.text_origin_x();
        let caret_x = origin + self.prefix_width(self.edit.cursor());
        let caret_h = self.cursor_plane.get_region().size.height;
        let caret_y = base.y + (self.region.size.height - caret_h) * 0.5;
        self.cursor_plane
            .set_position(Position::new(caret_x, caret_y, base.z + 2.0));
        match self.edit.selection().filter(|_| self.is_focused) {
            Some((start, end)) => {
                let x0 = origin + self.prefix_width(start);
                let x1 = origin + self.prefix_width(end);
                self.selection_plane
                    .set_position(Position::new(x0, caret_y, base.z + 1.0));
                self.selection_plane
                    .set_size(Size::new((x1 - x0).max(1.0), caret_h));
            }
            None => self.selection_plane.set_size(Size::new(0.0, 0.0)),
        }

        self.plane.border_color = if self.is_focused {
            theme::ACCENT
        } else if self.is_hovering {
            theme::BORDER_STRONG
        } else {
            theme::BORDER
        };
    }

    fn render(&self) {
        PlaneRenderer::render(&self.plane);
        crate::ui::clip::push(&self.stencil_plane);
        unsafe {
            gl::DepthFunc(gl::LEQUAL);
        }
        if self.is_focused {
            PlaneRenderer::render(&self.selection_plane);
        }
        self.text.render();
        // Blinking caret: 500 ms on, 500 ms off, clipped like the text.
        if self.is_focused && self.edit.selection().is_none() {
            let elapsed_ms = self.blink_start.elapsed().as_millis();
            if (elapsed_ms / 500) % 2 == 0 {
                PlaneRenderer::render(&self.cursor_plane);
            }
        }
        crate::ui::clip::pop(&self.stencil_plane);
    }

    fn handle_events(
        &mut self,
        _: &mut Scene,
        window: &mut glfw::Window,
        _: &mut glfw::Glfw,
        event: &WindowEvent,
    ) -> bool {
        match event {
            MouseButton(glfw::MouseButton::Button1, Press, mods) => {
                let (x, y) = window.get_cursor_pos();
                let (x, y) = (x as f32, y as f32);
                if !self.region.contains(x, y) {
                    if self.is_focused {
                        self.blur();
                    }
                    return false;
                }
                self.focus();
                let now = Instant::now();
                let clicks = match self.last_click {
                    Some((t, n)) if now.duration_since(t).as_secs_f32() < MULTI_CLICK_SECONDS => {
                        n + 1
                    }
                    _ => 1,
                };
                self.last_click = Some((now, clicks));
                let index = self.index_at(x);
                match clicks {
                    1 => {
                        self.edit
                            .set_cursor(index, mods.contains(Modifiers::Shift));
                        self.dragging = true;
                    }
                    2 => self.edit.select_word_at(index.saturating_sub(1)),
                    _ => self.edit.select_all(),
                }
                self.reset_blink();
                true
            }
            MouseButton(glfw::MouseButton::Button1, Release, _) => {
                let was = self.dragging;
                self.dragging = false;
                was
            }
            CursorPos(x, y) => {
                if self.dragging {
                    let index = self.index_at(*x as f32);
                    self.edit.set_cursor(index, true);
                    self.reset_blink();
                    return true;
                }
                if self.region.contains(*x as f32, *y as f32) {
                    if !self.is_hovering {
                        self.is_hovering = true;
                        window.set_cursor(Some(glfw::Cursor::standard(
                            glfw::StandardCursor::IBeam,
                        )));
                    }
                } else if self.is_hovering {
                    window.set_cursor(None);
                    self.is_hovering = false;
                }
                false
            }
            Char(character) if self.is_focused => {
                self.edit.insert(&character.to_string());
                self.commit();
                true
            }
            WindowEvent::Key(Key::Enter | Key::KpEnter | Key::Tab, _, Press, _)
                if self.is_focused =>
            {
                self.blur();
                true
            }
            WindowEvent::Key(Key::Escape, _, Press, _) if self.is_focused => {
                // Discard the unparsed remainder and show the bound value.
                self.blur();
                true
            }
            WindowEvent::Key(key, _, Press | Repeat, mods) if self.is_focused => {
                self.handle_key(window, *key, *mods)
            }
            _ => false,
        }
    }

    fn add_child(&mut self, _: Box<dyn UIElement>) {
        panic!("Input cannot have children");
    }

    fn get_handle(&self) -> &UIElementHandle {
        &self.handle
    }

    fn set_offset(&mut self, offset: Offset) {
        self.region.offset = offset;
        let p = &self.region.position + &self.region.offset;
        self.plane.set_position(p);
        self.stencil_plane.set_position(p);
    }

    fn get_size(&self) -> &Size {
        &self.region.size
    }

    fn contains_child(&self, _: &UIElementHandle) -> bool {
        false
    }

    fn get_child(&self, _handle: &UIElementHandle) -> Option<&Box<dyn UIElement>> {
        None
    }

    fn get_child_mut(&mut self, _handle: &UIElementHandle) -> Option<&mut Box<dyn UIElement>> {
        None
    }

    fn get_offset(&self) -> &Offset {
        &self.region.offset
    }

    fn add_child_to(&mut self, _: UIElementHandle, _: Box<dyn UIElement>) {
        panic!("Input cannot have children");
    }

    fn set_z_index(&mut self, z_index: f32) {
        self.region.position.z = z_index;
        self.plane.set_z_index(z_index);
        self.stencil_plane.set_z_index(z_index + 1.0);
        self.selection_plane.set_z_index(z_index + 1.0);
        self.text.set_z_index(z_index + 1.0);
        self.cursor_plane.set_z_index(z_index + 2.0);
    }
}
