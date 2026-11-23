//! The user's own editor settings (File > Settings), the same for every
//! project: kept in `$XDG_CONFIG_HOME/ferrite/editor.ron` (`~/.config`
//! without it, `%APPDATA%` on Windows). How a project's editor is arranged
//! lives in the project instead (`editor::project_settings`).

use std::{
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

use ferrite::serde::{Deserialize, Serialize};

use crate::i18n::{self, Language};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(crate = "ferrite::serde", default)]
pub struct Preferences {
    /// `"en"` or `"de"`; empty: the system's language.
    pub language: String,
    /// How fast the fly camera turns with the mouse (1: the old speed).
    pub mouse_sensitivity: f32,
    /// Moving the mouse up looks down.
    pub invert_y: bool,
    /// The fly camera's speed when the editor starts, in m/s (the mouse
    /// wheel changes it while flying).
    pub fly_speed: f32,
    /// How much faster the camera flies with Shift held.
    pub fast_factor: f32,
    /// The viewport camera's vertical field of view, in degrees.
    pub field_of_view: f32,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            language: String::new(),
            mouse_sensitivity: 1.0,
            invert_y: false,
            fly_speed: 20.0,
            fast_factor: 4.0,
            field_of_view: 64.0,
        }
    }
}

pub const MOUSE_SENSITIVITY: (f32, f32) = (0.2, 3.0);
pub const FLY_SPEED: (f32, f32) = (1.0, 200.0);
pub const FAST_FACTOR: (f32, f32) = (1.5, 10.0);
pub const FIELD_OF_VIEW: (f32, f32) = (40.0, 100.0);

impl Preferences {
    /// These preferences with every value in its range.
    fn sanitized(mut self) -> Self {
        let clamp = |v: f32, (min, max): (f32, f32), default: f32| if v.is_finite() { v.clamp(min, max) } else { default };
        let d = Preferences::default();
        self.mouse_sensitivity = clamp(self.mouse_sensitivity, MOUSE_SENSITIVITY, d.mouse_sensitivity);
        self.fly_speed = clamp(self.fly_speed, FLY_SPEED, d.fly_speed);
        self.fast_factor = clamp(self.fast_factor, FAST_FACTOR, d.fast_factor);
        self.field_of_view = clamp(self.field_of_view, FIELD_OF_VIEW, d.field_of_view);
        self
    }

    /// The language the editor speaks with these preferences.
    pub fn language(&self) -> Language {
        Language::from_code(&self.language).unwrap_or_else(Language::from_system)
    }
}

fn file_path() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    }?;
    Some(base.join("ferrite").join("editor.ron"))
}

fn cell() -> &'static Mutex<Preferences> {
    static CELL: OnceLock<Mutex<Preferences>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(Preferences::default()))
}

/// Read the preferences file (a missing or broken one is the defaults)
/// and speak its language.
pub fn load() {
    let preferences = file_path()
        .and_then(|path| match std::fs::read_to_string(&path) {
            Ok(text) => match ferrite::ron::from_str::<Preferences>(&text) {
                Ok(preferences) => Some(preferences),
                Err(e) => {
                    log::warn!("{}: {e}; using the defaults", path.display());
                    None
                }
            },
            Err(_) => None,
        })
        .unwrap_or_default()
        .sanitized();
    i18n::set_language(preferences.language());
    *cell().lock().unwrap() = preferences;
}

/// The preferences as they are now.
pub fn get() -> Preferences {
    cell().lock().unwrap().clone()
}

/// Use `preferences` from now on and save them.
pub fn set(preferences: Preferences) {
    let preferences = preferences.sanitized();
    i18n::set_language(preferences.language());
    {
        let mut current = cell().lock().unwrap();
        if *current == preferences {
            return;
        }
        *current = preferences.clone();
    }
    let Some(path) = file_path() else { return };
    let text = match ferrite::ron::ser::to_string_pretty(&preferences, Default::default()) {
        Ok(text) => text,
        Err(e) => return log::warn!("could not write the editor settings: {e}"),
    };
    if let Err(e) = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&path, text))
    {
        log::warn!("could not save the editor settings to {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_out_of_range_are_kept_in_it() {
        let wild = Preferences { mouse_sensitivity: 50.0, fly_speed: f32::NAN, ..Default::default() }.sanitized();
        assert_eq!(wild.mouse_sensitivity, MOUSE_SENSITIVITY.1);
        assert_eq!(wild.fly_speed, Preferences::default().fly_speed);
    }

    #[test]
    fn an_old_file_with_fewer_fields_still_reads() {
        let read: Preferences = ferrite::ron::from_str("(language: \"de\")").unwrap();
        assert_eq!(read.language(), Language::German);
        assert_eq!(read.fly_speed, 20.0);
    }
}
