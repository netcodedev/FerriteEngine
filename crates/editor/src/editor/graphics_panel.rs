//! The "Grafik" section of the Performance panel: the graphics settings
//! (docs/graphics-settings.md) with the frame time right above them. What
//! is set here is this machine's choice, saved like a player's
//! (`graphics::save`); the project's starting preset is set here too.

use ferrite::core::{
    graphics::{self, Level, SettingDef, SettingKind, SettingValue},
    primitives::{Position, Size},
    utils::DataSource,
};
use ferrite_ui::ui::{elements::dropdown::Dropdown, UIElement, UI};

use super::field_row::FieldRow;

/// Width of the labels' column: the settings' names are long.
const LABEL: f32 = 140.0;

const PRESET_LABELS: [&str; 5] = ["Niedrig", "Mittel", "Hoch", "Ultra", "Eigene"];
const PREVIEW_LABELS: [&str; 5] = ["Aus", "Niedrig", "Mittel", "Hoch", "Ultra"];

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
    Number(DataSource<f32>, f32),
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

    /// Make the rows, for a column `width` wide.
    pub fn build(&mut self, width: f32) -> Vec<Box<dyn UIElement>> {
        self.rows.clear();
        self.built_for = graphics::definitions();
        let value_w = (width - LABEL - 12.0).max(80.0);
        let settings = graphics::current();
        let mut widgets: Vec<Box<dyn UIElement>> = Vec::new();

        let dropdown = |rows: &mut Vec<Row>, widgets: &mut Vec<Box<dyn UIElement>>, label: &str, options: Vec<String>, index: u32, target: Target| {
            let source = Source::index(index);
            let Source::Index(data, _) = &source else { unreachable!() };
            let dropdown = Dropdown::new(Position::default(), Size::new(value_w, 24.0), options, data.clone());
            rows.push(Row { target, source });
            widgets.push(Box::new(FieldRow::with_label_width(label, LABEL, vec![Box::new(dropdown)])));
        };
        let strings = |labels: &[&str]| labels.iter().map(|l| l.to_string()).collect::<Vec<_>>();
        let preset = if settings.custom { 4 } else { level_index(settings.preset) };
        dropdown(&mut self.rows, &mut widgets, "Voreinstellung", strings(&PRESET_LABELS), preset, Target::Preset);
        let preview = settings.preview.map_or(0, |level| level_index(level) + 1);
        dropdown(&mut self.rows, &mut widgets, "Vorschau-Stufe", strings(&PREVIEW_LABELS), preview, Target::Preview);
        let project = level_index(graphics::project_settings().default_preset);
        dropdown(&mut self.rows, &mut widgets, "Projekt startet mit", strings(&PRESET_LABELS[..4]), project, Target::ProjectPreset);

        for def in self.built_for.clone() {
            let value = settings.value(&def.key).cloned();
            match (&def.kind, value) {
                (SettingKind::Level, Some(SettingValue::Level(level))) => {
                    let labels: Vec<String> = Level::ALL.iter().map(|l| l.label().to_string()).collect();
                    dropdown(&mut self.rows, &mut widgets, &def.label, labels, level_index(level), Target::Setting(def.clone()));
                }
                (SettingKind::Choice { options, .. }, Some(SettingValue::Choice(choice))) => {
                    let index = options.iter().position(|o| *o == choice).unwrap_or(0) as u32;
                    dropdown(&mut self.rows, &mut widgets, &def.label, options.clone(), index, Target::Setting(def.clone()));
                }
                (SettingKind::Toggle { .. }, Some(SettingValue::Toggle(on))) => {
                    let data = DataSource::new(on);
                    widgets.push(Box::new(FieldRow::with_label_width(&def.label, LABEL, vec![UI::checkbox(data.clone())])));
                    self.rows.push(Row { target: Target::Setting(def.clone()), source: Source::Bool(data, on) });
                }
                (SettingKind::Number { min, max, .. }, Some(SettingValue::Number(n))) => {
                    let data = DataSource::new(n);
                    let mut fields: Vec<Box<dyn UIElement>> = Vec::new();
                    if value_w > 150.0 {
                        fields.push(UI::slider_sized(data.clone(), *min, *max, Size::new(value_w - 74.0, 20.0)));
                        fields.push(UI::input_sized(data.clone(), Size::new(66.0, 24.0)));
                    } else {
                        fields.push(UI::input_sized(data.clone(), Size::new(value_w, 24.0)));
                    }
                    widgets.push(Box::new(FieldRow::with_label_width(&def.label, LABEL, fields)));
                    self.rows.push(Row { target: Target::Setting(def.clone()), source: Source::Number(data, n) });
                }
                _ => {}
            }
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
                (Target::Setting(def), Source::Number(data, last)) => {
                    let n = data.read();
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
                (Source::Number(data, last), SettingValue::Number(n), _) if (n - *last).abs() >= 1e-4 => {
                    *last = n;
                    data.write(n);
                }
                _ => {}
            }
        }
    }
}

fn set_index(data: &DataSource<u32>, last: &mut u32, index: u32) {
    if index != *last {
        *last = index;
        data.write(index);
    }
}
