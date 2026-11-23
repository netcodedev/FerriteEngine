//! File > Settings: the user's own editor settings (`crate::preferences`):
//! the language and how the viewport camera handles. The graphics settings
//! stay in the Performance panel.

use ferrite::core::{
    primitives::Size,
    utils::DataSource,
};
use ferrite_ui::ui::{
    elements::{dropdown::Dropdown, popup::Popup},
    theme, UIElement, UI,
};

use super::{dialogs::Modal, field_row::FieldRow, Shared};
use crate::{
    i18n::Language,
    preferences::{self, Preferences},
};

/// Width of the labels' column.
const LABEL: f32 = 190.0;
/// Width of the value widgets.
const VALUE: f32 = 180.0;

/// Show the settings dialog.
pub fn show(state: &Shared) {
    let current = preferences::get();
    let open = DataSource::new(true);

    let language_index = Language::ALL.iter().position(|l| *l == current.language()).unwrap_or(0) as u32;
    let language = DataSource::new(language_index);
    let sensitivity = DataSource::new(current.mouse_sensitivity);
    let invert_y = DataSource::new(current.invert_y);
    let fly_speed = DataSource::new(current.fly_speed);
    let fast_factor = DataSource::new(current.fast_factor);
    let field_of_view = DataSource::new(current.field_of_view);

    let heading = |text: &str| -> Box<dyn UIElement> {
        let mut label = UI::label(text, theme::FONT_SIZE);
        label.set_color(theme::TEXT);
        label
    };
    let number = |value: &DataSource<f32>| -> Vec<Box<dyn UIElement>> {
        vec![UI::input_sized(value.clone(), Size::new(VALUE, 22.0))]
    };
    let row = |label: &str, widgets: Vec<Box<dyn UIElement>>| -> Box<dyn UIElement> {
        Box::new(FieldRow::with_label_width(label, LABEL, widgets))
    };
    let range = |(min, max): (f32, f32)| tr!("settings.range", min = min, max = max);

    let languages: Vec<String> = Language::ALL.iter().map(|l| l.native_name().to_string()).collect();
    let language_box: Box<dyn UIElement> =
        Box::new(Dropdown::new(Default::default(), Size::new(VALUE, 24.0), languages, language.clone()));

    let apply = {
        let open = open.clone();
        let language = language.clone();
        let sensitivity = sensitivity.clone();
        let invert_y = invert_y.clone();
        let fly_speed = fly_speed.clone();
        let fast_factor = fast_factor.clone();
        let field_of_view = field_of_view.clone();
        Box::new(move |_: &mut ferrite::core::scene::Scene| {
            let chosen = Language::ALL.get(language.read() as usize).copied().unwrap_or(Language::English);
            preferences::set(Preferences {
                language: chosen.code().to_string(),
                mouse_sensitivity: sensitivity.read(),
                invert_y: invert_y.read(),
                fly_speed: fly_speed.read(),
                fast_factor: fast_factor.read(),
                field_of_view: field_of_view.read(),
            });
            open.write(false);
        })
    };
    let cancel = {
        let open = open.clone();
        Box::new(move |_: &mut ferrite::core::scene::Scene| open.write(false))
    };
    let reset = {
        let sensitivity = sensitivity.clone();
        let invert_y = invert_y.clone();
        let fly_speed = fly_speed.clone();
        let fast_factor = fast_factor.clone();
        let field_of_view = field_of_view.clone();
        Box::new(move |_: &mut ferrite::core::scene::Scene| {
            // Back to the defaults, the language aside.
            let d = Preferences::default();
            sensitivity.write(d.mouse_sensitivity);
            invert_y.write(d.invert_y);
            fly_speed.write(d.fly_speed);
            fast_factor.write(d.fast_factor);
            field_of_view.write(d.field_of_view);
        })
    };

    let mut buttons = UI::container_horizontal();
    buttons.add_child(UI::button(tr!("common.apply"), apply));
    buttons.add_child(UI::button(tr!("common.cancel"), cancel));
    buttons.add_child(UI::button(tr!("settings.defaults"), reset));

    let children: Vec<Box<dyn UIElement>> = vec![
        heading(tr!("settings.general")),
        row(tr!("settings.language"), vec![language_box]),
        heading(tr!("settings.camera")),
        row(&format!("{}  ({})", tr!("settings.mouse_sensitivity"), range(preferences::MOUSE_SENSITIVITY)), number(&sensitivity)),
        row(tr!("settings.invert_y"), vec![UI::checkbox(invert_y)]),
        row(&format!("{}  ({})", tr!("settings.fly_speed"), range(preferences::FLY_SPEED)), number(&fly_speed)),
        row(&format!("{}  ({})", tr!("settings.fast_factor"), range(preferences::FAST_FACTOR)), number(&fast_factor)),
        row(&format!("{}  ({})", tr!("settings.field_of_view"), range(preferences::FIELD_OF_VIEW)), number(&field_of_view)),
        UI::label(tr!("settings.graphics_hint"), theme::FONT_SIZE_SMALL),
        buttons,
    ];
    let popup = Popup::new(tr!("settings.title").to_owned(), open.clone(), children);
    state.borrow_mut().pending_modal = Some(Modal { open, popup });
}
