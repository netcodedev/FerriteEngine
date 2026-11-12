use crate::ui::theme;
use std::collections::BTreeMap;

use ferrite::core::{
    primitives::{Offset, Position, Region, Size},
    renderer::{plane::PlaneBuilder, text::{Fonts, Text}},
    scene::Scene,
};

use crate::ui::{element_handle::UIElementHandle, UIElement};

use glfw::{
    Action::Press,
    Glfw, MouseButtonLeft, Window,
    WindowEvent::{self, CursorPos, MouseButton},
};

use super::{Button, ButtonStyle};

const ICON_LABEL_GAP: f32 = 6.0;
const PADDING_X: f32 = 10.0;

impl Button {
    pub fn new(position: Position, size: Size, on_click: Box<dyn Fn(&mut Scene)>) -> Self {
        Self {
            handle: UIElementHandle::new(),

            region: Region::new_with_offset(position, size, Offset::default()),
            padding: Offset { x: 5.0, y: 1.0 },

            on_click,
            children: BTreeMap::new(),

            is_hovering: false,
            is_active: false,
            style: ButtonStyle::Secondary,

            label: None,
            icon: None,
            plane: PlaneBuilder::new()
                .position(position)
                .size(size)
                .border_radius_uniform(theme::RADIUS)
                .border_thickness(1.0)
                .border_color(theme::BORDER)
                .color(theme::BG_CONTROL)
                .build(),
        }
    }

    pub fn set_style(&mut self, style: ButtonStyle) {
        if self.style == style && self.plane.border_thickness == self.border_for(style) {
            return;
        }
        self.style = style;
        self.plane.border_thickness = self.border_for(style);
        self.refresh_colors();
    }

    fn border_for(&self, style: ButtonStyle) -> f32 {
        match style {
            ButtonStyle::Secondary => 1.0,
            _ => 0.0,
        }
    }

    /// Lucide icon drawn before the label (or alone, centred).
    pub fn set_icon(&mut self, glyph: &str) {
        match &mut self.icon {
            Some((current, _)) if current == glyph => {}
            Some((current, text)) => {
                *current = glyph.to_owned();
                text.set_content(glyph);
            }
            None => {
                let mut text = theme::icon(glyph, theme::ICON_SIZE, theme::TEXT);
                text.set_z_index(self.region.position.z + 1.0);
                self.icon = Some((glyph.to_owned(), text));
            }
        }
    }

    /// Width that fits icon and label with padding.
    pub fn content_width(&self) -> f32 {
        let icon = self.icon.as_ref().map_or(0.0, |(_, t)| t.measure_width());
        let label = self.label.as_ref().map_or(0.0, |(_, t)| t.measure_width());
        let gap = if icon > 0.0 && label > 0.0 { ICON_LABEL_GAP } else { 0.0 };
        let padding = if label > 0.0 { PADDING_X * 2.0 } else { 16.0 };
        (icon + gap + label + padding).ceil()
    }

    /// Resize to `content_width` at the given height.
    pub fn fit(&mut self, height: f32) {
        self.set_size(Size::new(self.content_width(), height));
    }

    fn refresh_colors(&mut self) {
        let fill = self.base_color();
        self.plane.set_color(fill);
        self.plane.border_color = if self.is_active && self.style == ButtonStyle::Secondary {
            theme::ACCENT
        } else {
            theme::BORDER
        };
        let text = self.text_color();
        if let Some((_, t)) = &mut self.label {
            t.set_color(text);
        }
        if let Some((_, t)) = &mut self.icon {
            t.set_color(text);
        }
    }

    fn text_color(&self) -> (f32, f32, f32, f32) {
        match (self.style, self.is_active, self.is_hovering) {
            (ButtonStyle::Primary, _, _) => (1.0, 1.0, 1.0, 1.0),
            (ButtonStyle::Danger, _, true) => (1.0, 1.0, 1.0, 1.0),
            (_, true, _) => theme::ACCENT_TEXT,
            (ButtonStyle::Ghost | ButtonStyle::Danger, false, false) => theme::TEXT_MUTED,
            _ => theme::TEXT,
        }
    }

    pub fn set_size(&mut self, size: Size) {
        self.region.size = size;
        self.plane.set_size(size);
    }

    /// Text drawn inside the button. Unlike a `Text` child, it can be
    /// changed later with `set_label` (e.g. "Play" ↔ "Stop").
    pub fn set_label(&mut self, label: &str) {
        match &mut self.label {
            Some((current, _)) if current == label => {}
            Some((current, text)) => {
                *current = label.to_owned();
                text.set_content(label);
            }
            None => {
                let mut text = Text::new(Fonts::Inter, 0, 0, 0, theme::FONT_SIZE, label.to_owned());
                text.set_z_index(self.region.position.z + 1.0);
                self.label = Some((label.to_owned(), text));
                self.refresh_colors();
            }
        }
    }

    /// Highlighted look for toggle-style buttons (selected tool, play mode).
    pub fn set_active(&mut self, active: bool) {
        if self.is_active != active {
            self.is_active = active;
            self.refresh_colors();
        }
    }

    fn base_color(&self) -> (f32, f32, f32, f32) {
        match (self.style, self.is_active, self.is_hovering) {
            (ButtonStyle::Primary, _, false) => theme::ACCENT,
            (ButtonStyle::Primary, _, true) => theme::ACCENT_HOVER,
            (ButtonStyle::Danger, _, true) => theme::DANGER,
            (_, true, _) => theme::ACCENT_SOFT,
            (ButtonStyle::Secondary, false, true) => theme::BG_CONTROL_HOVER,
            (ButtonStyle::Secondary, false, false) => theme::BG_CONTROL,
            (_, false, true) => theme::BG_ROW_HOVER,
            (_, false, false) => theme::TRANSPARENT,
        }
    }
}

impl UIElement for Button {
    fn update(&mut self, scene: &mut Scene) {
        let origin = self.region.get_absolute_position();
        let size = self.region.size;
        // Icon and label are laid out as one centred group.
        let icon_w = self.icon.as_ref().map_or(0.0, |(_, t)| t.measure_width());
        let label_w = self.label.as_ref().map_or(0.0, |(_, t)| t.measure_width());
        let gap = if icon_w > 0.0 && label_w > 0.0 { ICON_LABEL_GAP } else { 0.0 };
        let mut x = ((size.width - (icon_w + gap + label_w)) * 0.5).max(0.0).round();
        if let Some((_, icon)) = &mut self.icon {
            let y = theme::center_y(icon, size.height);
            icon.prepare_render_at(&origin + (x, y, 1.0));
            x += icon_w + gap;
        }
        if let Some((_, text)) = &mut self.label {
            let y = theme::center_y(text, size.height);
            text.prepare_render_at(&origin + (x, y, 1.0));
        }
        for child in self.children.values_mut() {
            child.update(scene);
        }
    }

    fn render(&self) {
        self.plane.render();
        if let Some((_, icon)) = &self.icon {
            icon.render();
        }
        if let Some((_, text)) = &self.label {
            text.render();
        }
        for child in self.children.values() {
            child.render();
        }
    }

    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut Window,
        _glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        match event {
            MouseButton(MouseButtonLeft, Press, _) => {
                let (x, y) = window.get_cursor_pos();
                let (x, y) = (x as f32, y as f32);
                if self.region.contains(x, y) {
                    (self.on_click)(scene);
                    return true;
                }
                false
            }
            CursorPos(x, y) => {
                if self.region.contains(*x as f32, *y as f32) {
                    if !self.is_hovering {
                        window.set_cursor(Some(glfw::Cursor::standard(glfw::StandardCursor::Hand)));
                        self.is_hovering = true;
                        self.refresh_colors();
                    }
                } else if self.is_hovering {
                    window.set_cursor(None);
                    self.is_hovering = false;
                    self.refresh_colors();
                }
                false
            }
            _ => false,
        }
    }

    fn add_child(&mut self, child: Box<dyn UIElement>) {
        self.children.insert(child.get_handle().clone(), child);
    }

    fn add_child_to(&mut self, parent: UIElementHandle, child: Box<dyn UIElement>) {
        if let Some(parent) = self.children.get_mut(&parent) {
            parent.add_child(child);
        } else {
            for child_elem in self.children.values_mut() {
                if child_elem.contains_child(&parent) {
                    child_elem.add_child_to(parent, child);
                    return;
                }
            }
        }
    }

    fn contains_child(&self, handle: &UIElementHandle) -> bool {
        if self.children.contains_key(handle) {
            return true;
        }
        for child in self.children.values() {
            if child.contains_child(handle) {
                return true;
            }
        }
        false
    }

    fn get_child(&self, handle: &UIElementHandle) -> Option<&Box<dyn UIElement>> {
        if let Some(child) = self.children.get(handle) {
            return Some(child);
        }
        for child in self.children.values() {
            if let Some(child) = child.get_child(handle) {
                return Some(child);
            }
        }
        None
    }

    fn get_child_mut(&mut self, handle: &UIElementHandle) -> Option<&mut Box<dyn UIElement>> {
        for (child_handle, child) in self.children.iter_mut() {
            if child_handle == handle {
                return Some(child);
            }
            if let Some(child) = child.get_child_mut(handle) {
                return Some(child);
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
        self.plane.set_position(&self.region.position + &self.region.offset);
        for child in self.children.values_mut() {
            child.set_offset(&(&self.region.offset + &self.padding) + &self.region.position);
        }
    }

    fn get_size(&self) -> &Size {
        &self.region.size
    }

    fn set_z_index(&mut self, z_index: f32) {
        self.region.position.z = z_index;
        self.plane.set_z_index(z_index);
        if let Some((_, text)) = &mut self.label {
            text.set_z_index(z_index + 1.0);
        }
        if let Some((_, icon)) = &mut self.icon {
            icon.set_z_index(z_index + 1.0);
        }
        for child in self.children.values_mut() {
            child.set_z_index(z_index + 1.0);
        }
    }
}
