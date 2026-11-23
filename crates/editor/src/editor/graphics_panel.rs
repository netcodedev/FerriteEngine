//! The "Graphics" section of the Performance panel: the graphics settings
//! (docs/graphics-settings.md) with the frame time right above them. What
//! is set here is this machine's choice, saved like a player's
//! (`graphics::save`); the project's starting preset is set here too.

use std::collections::BTreeMap;

use ferrite::core::{
    graphics::{self, Level, SettingDef, SettingKind, SettingValue},
    primitives::{Position, Size},
    utils::DataSource,
};
use ferrite_ui::ui::{
    elements::{dropdown::Dropdown, label::Label, section::Section},
    theme, UIElement, UI,
};

use super::field_row::FieldRow;
use crate::i18n;

/// Width of the labels' column: the settings' names are long.
const LABEL: f32 = 140.0;
/// Width of a slider's value beside it.
const VALUE: f32 = 56.0;

/// A level's name in the user's language (the engine's `Level::label`
/// is German, for games).
fn level_label(level: Level) -> &'static str {
    match level {
        Level::Low => tr!("graphics.level.low"),
        Level::Medium => tr!("graphics.level.medium"),
        Level::High => tr!("graphics.level.high"),
        Level::Ultra => tr!("graphics.level.ultra"),
    }
}

/// The presets' names; with `custom`, "Custom" for settings changed one
/// by one.
fn preset_labels(custom: bool) -> Vec<String> {
    let mut labels: Vec<String> = Level::ALL.iter().map(|l| level_label(*l).to_string()).collect();
    if custom {
        labels.push(tr!("graphics.custom").to_string());
    }
    labels
}

/// The preview levels: off, then each level.
fn preview_labels() -> Vec<String> {
    std::iter::once(tr!("common.off").to_string())
        .chain(Level::ALL.iter().map(|l| level_label(*l).to_string()))
        .collect()
}

/// A setting's name: the editor's text for the engine's settings, the
/// project's own label for the project's.
fn setting_label(def: &SettingDef) -> &str {
    i18n::lookup(&format!("graphics.setting.{}", def.key)).unwrap_or(&def.label)
}

/// A group's name, the same way (groups are named by their German title).
fn group_label(group: &str) -> String {
    i18n::lookup(&format!("graphics.group.{}", group.to_lowercase()))
        .map_or_else(|| group.to_string(), str::to_string)
}

/// A choice's option as shown (`"off"` translated, `"4x"` as it is).
fn option_label(option: &str) -> String {
    match option {
        "off" => tr!("common.off").to_string(),
        option => option.to_string(),
    }
}

fn level_index(level: Level) -> u32 {
    Level::ALL.iter().position(|l| *l == level).unwrap_or(2) as u32
}

fn level_at(index: u32) -> Option<Level> {
    Level::ALL.get(index as usize).copied()
}

/// A widget's value and the value last shown in it.
enum Source {
    Index(DataSource<u32>, u32),
    Bool(DataSource<bool>, bool),
    /// A slider, and the text showing its value.
    Number(DataSource<f32>, f32, DataSource<String>),
}

impl Source {
    fn index(value: u32) -> Self {
        Source::Index(DataSource::new(value), value)
    }
}

/// What a row sets.
enum Target {
    Preset,
    Preview,
    ProjectPreset,
    Setting(SettingDef),
}

struct Row {
    target: Target,
    source: Source,
}

/// The rows' sources, kept by the Performance panel between frames.
#[derive(Default)]
pub struct GraphicsRows {
    rows: Vec<Row>,
    /// The definitions the rows were made for: other ones (the project
    /// changed them) need other rows.
    built_for: Vec<SettingDef>,
}

impl GraphicsRows {
    /// Whether the rows have to be made again.
    pub fn outdated(&self) -> bool {
        graphics::definitions() != self.built_for
    }

    /// Make the rows, for a column `width` wide: the presets on top, then
    /// one foldable section per group of settings, folded as `open` says
    /// (by group id; groups not in it start open and are added).
    pub fn build(&mut self, width: f32, open: &mut BTreeMap<String, DataSource<bool>>) -> Vec<Box<dyn UIElement>> {
        self.rows.clear();
        self.built_for = graphics::definitions();
        let settings = graphics::current();
        let mut widgets: Vec<Box<dyn UIElement>> = Vec::new();

        let dropdown = |rows: &mut Vec<Row>, width: f32, label: &str, options: Vec<String>, index: u32, target: Target| -> Box<dyn UIElement> {
            let value_w = (width - LABEL - 12.0).max(80.0);
            let source = Source::index(index);
            let Source::Index(data, _) = &source else { unreachable!() };
            let dropdown = Dropdown::new(Position::default(), Size::new(value_w, 24.0), options, data.clone());
            rows.push(Row { target, source });
            Box::new(FieldRow::with_label_width(label, LABEL, vec![Box::new(dropdown)]))
        };
        let preset = if settings.custom { 4 } else { level_index(settings.preset) };
        widgets.push(dropdown(&mut self.rows, width, tr!("graphics.preset"), preset_labels(true), preset, Target::Preset));
        let preview = settings.preview.map_or(0, |level| level_index(level) + 1);
        widgets.push(dropdown(&mut self.rows, width, tr!("graphics.preview"), preview_labels(), preview, Target::Preview));
        let project = level_index(graphics::project_settings().default_preset);
        widgets.push(dropdown(&mut self.rows, width, tr!("graphics.project_preset"), preset_labels(false), project, Target::ProjectPreset));

        // The groups in the order their first setting comes.
        let mut groups: Vec<(String, Vec<SettingDef>)> = Vec::new();
        for def in self.built_for.clone() {
            match groups.iter_mut().find(|(group, _)| *group == def.group) {
                Some((_, defs)) => defs.push(def),
                None => groups.push((def.group.clone(), vec![def])),
            }
        }
        // The sections' rows are set in a little.
        let inner = width - 8.0;
        for (group, defs) in groups {
            let mut section = Section::new(&group_label(&group), None, width);
            section.bind_open(open.entry(group.clone()).or_insert_with(|| DataSource::new(true)).clone());
            for def in defs {
                let value = settings.value(&def.key).cloned();
                let label = setting_label(&def);
                let value_w = (inner - LABEL - 12.0).max(80.0);
                match (&def.kind, value) {
                    (SettingKind::Level, Some(SettingValue::Level(level))) => {
                        let labels = Level::ALL.iter().map(|l| level_label(*l).to_string()).collect();
                        section.add_child(dropdown(&mut self.rows, inner, label, labels, level_index(level), Target::Setting(def.clone())));
                    }
                    (SettingKind::Choice { options, .. }, Some(SettingValue::Choice(choice))) => {
                        let index = options.iter().position(|o| *o == choice).unwrap_or(0) as u32;
                        let shown = options.iter().map(|o| option_label(o)).collect();
                        section.add_child(dropdown(&mut self.rows, inner, label, shown, index, Target::Setting(def.clone())));
                    }
                    (SettingKind::Toggle { .. }, Some(SettingValue::Toggle(on))) => {
                        let data = DataSource::new(on);
                        // A checkbox is small: its label gets the rest of the row.
                        let label_w = (inner - 30.0).max(LABEL);
                        section.add_child(Box::new(FieldRow::with_label_width(label, label_w, vec![UI::checkbox(data.clone())])));
                        self.rows.push(Row { target: Target::Setting(def.clone()), source: Source::Bool(data, on) });
                    }
                    (SettingKind::Number { min, max, unit, .. }, Some(SettingValue::Number(n))) => {
                        // A slider and its value: a typed number would be set
                        // (and clamped) on every key.
                        let data = DataSource::new(n);
                        let shown = DataSource::new(number_text(n, unit));
                        let mut value = Label::bound(shown.clone(), theme::FONT_SIZE_SMALL);
                        value.set_color(theme::TEXT_MUTED);
                        let fields: Vec<Box<dyn UIElement>> = vec![
                            UI::slider_sized(data.clone(), *min, *max, Size::new((value_w - VALUE - 4.0).max(40.0), 20.0)),
                            Box::new(value),
                        ];
                        section.add_child(Box::new(FieldRow::with_label_width(label, LABEL, fields)));
                        self.rows.push(Row { target: Target::Setting(def.clone()), source: Source::Number(data, n, shown) });
                    }
                    _ => {}
                }
            }
            widgets.push(Box::new(section));
        }
        widgets
    }

    /// Hand what was changed in the rows to the settings, and show what
    /// the settings hold now (changed by a preset, MCP or a script).
    pub fn sync(&mut self) {
        // What was changed here first.
        for row in &mut self.rows {
            let result = match (&row.target, &mut row.source) {
                (target, Source::Index(data, last)) => {
                    let index = data.read();
                    if index == *last {
                        continue;
                    }
                    *last = index;
                    match target {
                        Target::Preset => {
                            if let Some(level) = level_at(index) {
                                graphics::set_preset(level);
                            }
                            Ok(())
                        }
                        Target::Preview => {
                            graphics::set_preview(index.checked_sub(1).and_then(level_at));
                            Ok(())
                        }
                        Target::ProjectPreset => {
                            if let Some(level) = level_at(index) {
                                let mut project = graphics::project_settings();
                                project.default_preset = level;
                                graphics::set_project_settings(&project);
                            }
                            Ok(())
                        }
                        Target::Setting(def) => match &def.kind {
                            SettingKind::Level => match level_at(index) {
                                Some(level) => graphics::set(&def.key, SettingValue::Level(level)),
                                None => Ok(()),
                            },
                            SettingKind::Choice { options, .. } => match options.get(index as usize) {
                                Some(option) => graphics::set(&def.key, SettingValue::Choice(option.clone())),
                                None => Ok(()),
                            },
                            _ => Ok(()),
                        },
                    }
                }
                (Target::Setting(def), Source::Bool(data, last)) => {
                    let on = data.read();
                    if on == *last {
                        continue;
                    }
                    *last = on;
                    graphics::set(&def.key, SettingValue::Toggle(on))
                }
                (Target::Setting(def), Source::Number(data, last, _)) => {
                    // Kept to the setting's steps, so dragging changes it
                    // (and what depends on it) only step by step.
                    let n = snapped(&def.kind, data.read());
                    if (n - *last).abs() < 1e-4 {
                        continue;
                    }
                    *last = n;
                    graphics::set(&def.key, SettingValue::Number(n))
                }
                _ => Ok(()),
            };
            if let Err(e) = result {
                log::warn!("{e}");
            }
        }
        // Then what the settings hold.
        let settings = graphics::current();
        for row in &mut self.rows {
            let shown = match &row.target {
                Target::Preset => Some(SettingValue::Number(if settings.custom { 4.0 } else { level_index(settings.preset) as f32 })),
                Target::Preview => Some(SettingValue::Number(settings.preview.map_or(0, |l| level_index(l) + 1) as f32)),
                Target::ProjectPreset => {
                    Some(SettingValue::Number(level_index(graphics::project_settings().default_preset) as f32))
                }
                Target::Setting(def) => settings.value(&def.key).cloned(),
            };
            let Some(shown) = shown else { continue };
            match (&mut row.source, shown, &row.target) {
                (Source::Index(data, last), SettingValue::Number(index), _) => {
                    set_index(data, last, index as u32);
                }
                (Source::Index(data, last), SettingValue::Level(level), _) => set_index(data, last, level_index(level)),
                (Source::Index(data, last), SettingValue::Choice(choice), Target::Setting(def)) => {
                    if let SettingKind::Choice { options, .. } = &def.kind {
                        let index = options.iter().position(|o| *o == choice).unwrap_or(0) as u32;
                        set_index(data, last, index);
                    }
                }
                (Source::Bool(data, last), SettingValue::Toggle(on), _) if on != *last => {
                    *last = on;
                    data.write(on);
                }
                (Source::Number(data, last, shown), SettingValue::Number(n), Target::Setting(def)) => {
                    if (n - *last).abs() >= 1e-4 {
                        *last = n;
                        data.write(n);
                    }
                    if let SettingKind::Number { unit, .. } = &def.kind {
                        let text = number_text(n, unit);
                        if shown.read() != text {
                            shown.write(text);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

/// `value` on the steps of a number setting.
fn snapped(kind: &SettingKind, value: f32) -> f32 {
    match kind {
        SettingKind::Number { min, max, step, .. } if *step > 0.0 => {
            (min + ((value - min) / step).round() * step).clamp(*min, *max)
        }
        _ => value,
    }
}

/// A number setting's value as shown beside its slider (set in a little:
/// the slider's knob reaches past its end).
fn number_text(value: f32, unit: &str) -> String {
    let text = match unit {
        "fps" if value == 0.0 => tr!("graphics.no_limit").into(),
        "%" => format!("{value:.0} %"),
        unit => format!("{} {unit}", (value * 10.0).round() / 10.0),
    };
    format!("   {text}")
}

fn set_index(data: &DataSource<u32>, last: &mut u32, index: u32) {
    if index != *last {
        *last = index;
        data.write(index);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_engine_setting_and_group_has_a_text() {
        for def in ferrite::core::graphics::engine_definitions() {
            assert!(i18n::lookup(&format!("graphics.setting.{}", def.key)).is_some(), "no text for {}", def.key);
            assert!(i18n::lookup(&format!("graphics.group.{}", def.group.to_lowercase())).is_some(), "no text for group {}", def.group);
        }
    }
}
