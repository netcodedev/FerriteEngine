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
            closed_stats_sections: Vec::new(),
            window: None,
            maximized: false,
        }
    }
}

impl EditorSettings {
    /// What the project file holds, or the defaults.
    pub fn from_value(value: &ferrite::ron::Value) -> Self {
        match value {
            ferrite::ron::Value::Unit => Self::default(),
            value => value.clone().into_rust().unwrap_or_default(),
        }
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
        for (title, open) in super::stats::section_titles().zip(&state.stats_open) {
            open.write(!self.closed_stats_sections.iter().any(|closed| closed == title));
        }
    }
}
