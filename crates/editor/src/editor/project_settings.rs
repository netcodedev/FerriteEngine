//! How the user arranged the editor, kept in the project file: which
//! windows are shown, how large the panels are, how large the window is.

use ferrite::serde::{Deserialize, Serialize};

use super::{layout::PanelSizes, EditorState};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(crate = "ferrite::serde", default)]
pub struct EditorSettings {
    pub panels: PanelSizes,
    pub show_stats: bool,
    pub show_widgets: bool,
    pub show_assets: bool,
    pub show_console: bool,
    /// The console's level and source filters.
    pub console: super::console::ConsoleFilter,
    /// The asset browser's category (`view::Category::key`) and open folder
    /// (below the asset root, `/`-separated).
    pub asset_category: String,
    pub asset_folder: String,
    /// The sections of the Performance panel that are folded, by title.
    pub closed_stats_sections: Vec<String>,
    /// Window size in pixels; `None` keeps the default.
    pub window: Option<(u32, u32)>,
    pub maximized: bool,
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            panels: PanelSizes::default(),
            show_stats: true,
            show_widgets: false,
            show_assets: true,
            show_console: true,
            console: super::console::ConsoleFilter::default(),
            asset_category: String::new(),
            asset_folder: String::new(),
            closed_stats_sections: Vec::new(),
            window: None,
            maximized: false,
        }
    }
}

/// Smallest window size restored from the project file, so a bad saved size
/// (from a minimized window, say) does not leave the editor unusable.
pub const MIN_WINDOW: (u32, u32) = (640, 360);

impl EditorSettings {
    /// What the project file holds, or the defaults.
    pub fn from_value(value: &ferrite::ron::Value) -> Self {
        let settings: Self = match value {
            ferrite::ron::Value::Unit => Self::default(),
            value => value.clone().into_rust().unwrap_or_default(),
        };
        settings.sanitized()
    }

    /// These settings with sizes the editor could not be used with replaced.
    fn sanitized(mut self) -> Self {
        self.panels = self.panels.sanitized();
        self.window = self
            .window
            .map(|(width, height)| (width.max(MIN_WINDOW.0), height.max(MIN_WINDOW.1)));
        self
    }

    pub fn to_value(&self) -> ferrite::ron::Value {
        ferrite::ron::to_string(self)
            .ok()
            .and_then(|text| ferrite::ron::from_str(&text).ok())
            .unwrap_or(ferrite::ron::Value::Unit)
    }

    /// The editor's arrangement as it is now (window size aside).
    pub fn capture(state: &EditorState, window: Option<(u32, u32)>, maximized: bool) -> Self {
        Self {
            panels: state.panel_sizes,
            show_stats: state.show_stats.read(),
            show_widgets: state.show_widgets.read(),
            show_assets: state.show_assets.read(),
            show_console: state.show_console.read(),
            console: state.console.clone(),
            asset_category: state.asset_view.0.clone(),
            asset_folder: state.asset_view.1.clone(),
            closed_stats_sections: super::stats::section_titles()
                .zip(&state.stats_open)
                .filter(|(_, open)| !open.read())
                .map(|(title, _)| title.to_string())
                .collect(),
            window,
            maximized,
        }
    }

    /// Arrange the editor like this.
    pub fn apply(&self, state: &mut EditorState) {
        state.panel_sizes = self.panels;
        state.show_stats.write(self.show_stats);
        state.show_widgets.write(self.show_widgets);
        state.show_assets.write(self.show_assets);
        state.show_console.write(self.show_console);
        state.console = self.console.clone();
        state.asset_view = (self.asset_category.clone(), self.asset_folder.clone());
        for (title, open) in super::stats::section_titles().zip(&state.stats_open) {
            open.write(!self.closed_stats_sections.iter().any(|closed| closed == title));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tiny_saved_window_opens_at_a_usable_size() {
        let saved = EditorSettings {
            window: Some((1, 1)),
            ..EditorSettings::default()
        };
        let loaded = EditorSettings::from_value(&saved.to_value());
        assert_eq!(loaded.window, Some(MIN_WINDOW));
    }
}
