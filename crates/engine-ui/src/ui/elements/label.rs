use ferrite::core::{
    primitives::{Offset, Region, Size},
    renderer::text::{Fonts, Text as FerriteText},
    scene::Scene,
    utils::DataSource,
};
use glfw::{Glfw, Window, WindowEvent};

use crate::ui::{element_handle::UIElementHandle, UIElement};

/// Static or data-bound text label. Non-interactive.
pub struct Label {
    handle: UIElementHandle,
    region: Region,
    content: String,
    source: Option<DataSource<String>>,
    text: FerriteText,
    /// Longer text is cut with "…".
    max_width: Option<f32>,
    /// Content and width the shown text was cut for.
    fitted_for: Option<(String, Option<f32>)>,
}

impl Label {
    pub fn new(text: impl Into<String>, size: f32) -> Self {
        let content: String = text.into();
        Self {
            handle: UIElementHandle::new(),
            region: Region {
                size: Size::new(size * content.len() as f32, size),
                ..Default::default()
            },
            text: {
                let mut t = FerriteText::new(Fonts::Inter, 0, 0, 0, size, content.clone());
                t.set_color(crate::ui::theme::TEXT);
                t
            },
            content,
            source: None,
            max_width: None,
            fitted_for: None,
        }
    }

    pub fn bound(source: DataSource<String>, size: f32) -> Self {
        let mut l = Self::new(source.read(), size);
        l.source = Some(source);
        l
    }

    pub fn set_color(&mut self, color: (f32, f32, f32, f32)) {
        self.text.set_color(color);
    }

    pub fn set_font(&mut self, font: Fonts, size: f32) {
        self.text.set_font(font, size);
        self.fitted_for = None;
    }

    /// Dimmed secondary text.
    pub fn muted(text: impl Into<String>, size: f32) -> Self {
        let mut label = Self::new(text, size);
        label.set_color(crate::ui::theme::TEXT_MUTED);
        label
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.content = text.into();
        self.refit();
    }

    /// Show the content cut to `max_width`, unless it already is.
    fn refit(&mut self) {
        if let Some((content, width)) = &self.fitted_for {
            if *content == self.content && *width == self.max_width {
                return;
            }
        }
        let shown = self.fitted();
        self.text.set_content(&shown);
        self.fitted_for = Some((self.content.clone(), self.max_width));
    }

    /// Cut the text with "…" where it gets wider than `width`; `None`
    /// shows all of it.
    pub fn set_max_width(&mut self, width: Option<f32>) {
        self.max_width = width;
    }

    /// The content as shown: cut to `max_width`.
    fn fitted(&self) -> String {
        let Some(max) = self.max_width else {
            return self.content.clone();
        };
        if self.text.measure_str(&self.content) <= max {
            return self.content.clone();
        }
        let chars: Vec<char> = self.content.chars().collect();
        // The longest prefix that fits with the ellipsis.
        let (mut low, mut high) = (0, chars.len());
        while low < high {
            let mid = (low + high + 1) / 2;
            let candidate: String = chars[..mid].iter().chain(['\u{2026}'].iter()).collect();
            if self.text.measure_str(&candidate) <= max {
                low = mid;
            } else {
                high = mid - 1;
            }
        }
        if low == 0 {
            return String::new();
        }
        chars[..low].iter().chain(['\u{2026}'].iter()).collect()
    }
}

impl UIElement for Label {
    fn update(&mut self, _scene: &mut Scene) {
        if let Some(src) = &self.source {
            self.content = src.read();
        }
        self.refit();
        self.text.prepare_render_at(self.region.get_absolute_position());
        self.region.size = self.text.get_size();
    }

    fn render(&self) {
        self.text.render();
    }

    fn handle_events(
        &mut self,
        _scene: &mut Scene,
        _window: &mut Window,
        _glfw: &mut Glfw,
        _event: &WindowEvent,
    ) -> bool {
        false
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
    }
    fn get_size(&self) -> &Size {
        &self.region.size
    }
    fn set_z_index(&mut self, z_index: f32) {
        self.region.position.z = z_index;
        self.text.set_z_index(z_index);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cut_text_follows_content_width_and_font() {
        ferrite::core::headless::enable();
        let mut scene = Scene::new();
        let long = "a rather long label that does not fit";
        let mut label = Label::new(long, 14.0);
        label.set_max_width(Some(80.0));
        label.update(&mut scene);
        let cut = label.text.content.clone();
        assert!(cut.ends_with('\u{2026}'), "{cut}");

        label.set_text("short");
        assert_eq!(label.text.content, "short");
        label.set_text(long);
        assert_eq!(label.text.content, cut);

        label.set_max_width(None);
        label.update(&mut scene);
        assert_eq!(label.text.content, long);

        label.set_max_width(Some(80.0));
        label.update(&mut scene);
        assert_eq!(label.text.content, cut);
        // A bigger font fits fewer chars.
        label.set_font(Fonts::Inter, 20.0);
        label.update(&mut scene);
        assert!(label.text.content.chars().count() < cut.chars().count());
    }
}
