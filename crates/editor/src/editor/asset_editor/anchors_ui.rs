//! The inspector's sections for anchors and a clip's references (see
//! `docs/anchors.md`): which models a clip shows with it and how, and the
//! fields of the selected anchor.

use ferrite::core::{
    assets::{self, AssetId},
    model::{anchor::ReferenceLink, clip_file::ClipReference, ik::base_name},
    primitives::{Position, Size},
    renderer::text::icons,
    utils::DataSource,
};
use ferrite_ui::ui::{
    elements::{
        button::{Button, ButtonStyle},
        dropdown::Dropdown,
        label::Label,
        section::Section,
    },
    theme, UIElement, UI,
};

use crate::editor::{field_row::FieldRow, Shared};

use super::*;

/// The widgets of one reference.
pub(super) struct RefBindings {
    index: usize,
    model: DataSource<u32>,
    link: DataSource<u32>,
    /// 0 is the origin, then the anchors by name.
    anchor: DataSource<u32>,
    model_anchor: DataSource<u32>,
    models: Vec<AssetId>,
    anchors: Vec<String>,
    model_anchors: Vec<String>,
    opacity: DataSource<f32>,
    /// The dropdowns' values as last seen: only a changed one is written
    /// back (a renamed anchor must not be set back to its old name).
    seen: [u32; 4],
}

impl RefBindings {
    fn now(&self) -> [u32; 4] {
        [self.model.read(), self.link.read(), self.anchor.read(), self.model_anchor.read()]
    }
}

/// The fields of the selected anchor, with the values they were last seen
/// at: only what is typed is written back.
pub(super) struct AnchorBindings {
    pick: AnchorPick,
    name: DataSource<String>,
    bone: DataSource<String>,
    values: [DataSource<f32>; 6],
    seen: ([f32; 6], String, String),
}

impl AnchorBindings {
    fn now(&self) -> ([f32; 6], String, String) {
        (
            [0, 1, 2, 3, 4, 5].map(|i| self.values[i].read()),
            self.name.read(),
            self.bone.read(),
        )
    }
}

fn muted(text: impl Into<String>) -> Box<dyn UIElement> {
    Box::new(Label::muted(text.into(), theme::FONT_SIZE_SMALL + 0.5))
}

fn asset_name(id: AssetId) -> String {
    assets::database()
        .get(id)
        .map(|entry| entry.meta.name.clone())
        .unwrap_or_else(|| format!("missing ({id})"))
}

fn small_button(label: &str, icon: &str, style: ButtonStyle, on_click: Box<dyn Fn(&mut ferrite::core::scene::Scene)>) -> Button {
    let mut button = Button::new(Position::default(), Size::new(10.0, 24.0), on_click);
    button.set_style(style);
    button.set_icon(icon);
    button.set_label(label);
    button.fit(24.0);
    button
}

/// Run `f` on the open asset edit.
fn with_edit(state: &Shared, f: impl FnOnce(&mut AssetEdit)) {
    if let Some(edit) = state.borrow_mut().asset_edit.as_mut() {
        f(edit);
    }
}

/// A new reference: the first model that is not the clip's, held by its
/// grip if it has one, else carrying the model.
fn new_reference(edit: &AssetEdit) -> Option<ClipReference> {
    let clip_model = edit.clip_model();
    let model = model_assets()
        .into_iter()
        .map(|(id, _)| id)
        .find(|id| Some(*id) != clip_model)?;
    Some(ClipReference {
        model,
        link: default_link(edit, model),
        anchor: None,
        model_anchor: None,
        opacity: 1.0,
    })
}

fn default_link(edit: &AssetEdit, model: AssetId) -> ReferenceLink {
    if edit.grip_of(model).is_some() {
        ReferenceLink::Held(ferrite::core::model::grip::Hand::Right)
    } else {
        ReferenceLink::Carries
    }
}

/// The clip's references, each with its model, link and anchors.
pub(super) fn references_section(
    state: &Shared,
    edit: &AssetEdit,
    width: f32,
    value_w: f32,
) -> Option<(Section, Vec<RefBindings>)> {
    let clip = edit.clip.as_ref()?;
    let clip_model = edit.clip_model();
    let models = model_assets();
    let model_names: Vec<String> = models.iter().map(|(_, name)| name.clone()).collect();
    let model_ids: Vec<AssetId> = models.iter().map(|(id, _)| *id).collect();
    let links: Vec<String> = ReferenceLink::ALL.iter().map(|l| l.label().to_string()).collect();
    let model_anchors: Vec<String> = clip_model
        .map(|id| edit.anchors(id).into_iter().map(|a| a.name).collect())
        .unwrap_or_default();
    let with_origin = |names: &[String]| {
        let mut out = vec!["origin".to_string()];
        out.extend(names.iter().cloned());
        out
    };
    let index_of = |names: &[String], name: Option<&str>| {
        name.and_then(|name| names.iter().position(|n| n == name))
            .map_or(0, |at| at as u32 + 1)
    };

    let mut section = Section::new("References", Some(icons::BOXES), width);
    let mut bindings = Vec::new();
    for (index, reference) in clip.file.references.iter().enumerate() {
        let anchors: Vec<String> = edit.anchors(reference.model).into_iter().map(|a| a.name).collect();
        let mut b = RefBindings {
            index,
            model: DataSource::new(model_ids.iter().position(|id| *id == reference.model).unwrap_or(0) as u32),
            link: DataSource::new(ReferenceLink::ALL.iter().position(|l| *l == reference.link).unwrap_or(0) as u32),
            anchor: DataSource::new(index_of(&anchors, reference.anchor.as_deref())),
            model_anchor: DataSource::new(index_of(&model_anchors, reference.model_anchor.as_deref())),
            models: model_ids.clone(),
            anchors: anchors.clone(),
            model_anchors: model_anchors.clone(),
            opacity: DataSource::new(reference.opacity),
            seen: [0; 4],
        };
        b.seen = b.now();
        let remove_state = state.clone();
        let mut remove = small_button(
            "",
            icons::X,
            ButtonStyle::Danger,
            Box::new(move |_| {
                with_edit(&remove_state, |edit| {
                    if let Some(clip) = edit.clip.as_mut() {
                        clip.remove_reference(index);
                        edit.dirty = true;
                        edit.structure += 1;
                    }
                })
            }),
        );
        remove.set_tooltip("Stop showing this model with the clip");
        section.add_child(Box::new(FieldRow::new(
            &format!("Model {}", index + 1),
            vec![
                Box::new(Dropdown::new(
                    Position::default(),
                    Size::new((value_w - 30.0).max(60.0), 24.0),
                    model_names.clone(),
                    b.model.clone(),
                )),
                Box::new(remove),
            ],
        )));
        section.add_child(Box::new(FieldRow::new(
            "Link",
            vec![Box::new(Dropdown::new(
                Position::default(),
                Size::new(value_w, 24.0),
                links.clone(),
                b.link.clone(),
            ))],
        )));
        if reference.link.uses_anchors() {
            section.add_child(Box::new(FieldRow::new(
                "Its anchor",
                vec![Box::new(Dropdown::new(
                    Position::default(),
                    Size::new(value_w, 24.0),
                    with_origin(&anchors),
                    b.anchor.clone(),
                ))],
            )));
            section.add_child(Box::new(FieldRow::new(
                "On the model's",
                vec![Box::new(Dropdown::new(
                    Position::default(),
                    Size::new(value_w, 24.0),
                    with_origin(&model_anchors),
                    b.model_anchor.clone(),
                ))],
            )));
        }
        section.add_child(Box::new(FieldRow::new(
            "Opacity",
            vec![UI::slider_sized(b.opacity.clone(), 0.0, 1.0, Size::new(value_w, 24.0))],
        )));
        if !reference.link.uses_anchors() && edit.grip_of(reference.model).is_none() {
            section.add_child(muted(
                "This model has no grip yet: give it one in its asset settings (Grip).",
            ));
        }
        bindings.push(b);
    }
    let add_state = state.clone();
    let mut add = small_button(
        "Add reference",
        icons::PLUS,
        ButtonStyle::Secondary,
        Box::new(move |_| {
            with_edit(&add_state, |edit| {
                let Some(reference) = new_reference(edit) else { return };
                if let Some(clip) = edit.clip.as_mut() {
                    clip.add_reference(reference);
                    edit.dirty = true;
                    edit.structure += 1;
                }
            })
        }),
    );
    add.set_tooltip("Show another model with the clip: an item in the hand, a seat to sit on");
    section.add_child(Box::new(add));
    if clip.file.references.is_empty() {
        section.add_child(muted(
            "Other models to animate against. They are saved with the clip and only shown in the editor.",
        ));
    }
    Some((section, bindings))
}

/// Carry what was picked for the references over to the clip; whether
/// the panel has to be built again (other choices to offer).
pub(super) fn sync_references(edit: &mut AssetEdit, refs: &mut [RefBindings]) -> bool {
    let mut rebuild = false;
    let mut changed = false;
    for b in refs {
        let now = b.now();
        let Some(old) = edit.clip.as_ref().and_then(|clip| clip.file.references.get(b.index)).cloned() else {
            continue;
        };
        if now == b.seen {
            let opacity = b.opacity.read().clamp(0.0, 1.0);
            if (opacity - old.opacity).abs() > 1e-4 {
                if let Some(clip) = edit.clip.as_mut() {
                    changed |= clip.set_reference(b.index, ClipReference { opacity, ..old });
                }
            }
            continue;
        }
        let seen = std::mem::replace(&mut b.seen, now);
        let mut new = old.clone();
        let pick = |names: &[String], at: u32| at.checked_sub(1).and_then(|i| names.get(i as usize).cloned());
        if now[0] != seen[0] {
            if let Some(&model) = b.models.get(now[0] as usize) {
                // Its anchors are another model's; held if it can be.
                new.model = model;
                new.anchor = None;
                new.link = default_link(edit, model);
            }
        }
        if now[1] != seen[1] {
            if let Some(&link) = ReferenceLink::ALL.get(now[1] as usize) {
                new.link = link;
            }
        }
        if now[2] != seen[2] {
            new.anchor = pick(&b.anchors, now[2]);
        }
        if now[3] != seen[3] {
            new.model_anchor = pick(&b.model_anchors, now[3]);
        }
        // Other choices to offer.
        rebuild |= new.model != old.model || new.link != old.link;
        new.opacity = b.opacity.read().clamp(0.0, 1.0);
        if let Some(clip) = edit.clip.as_mut() {
            changed |= clip.set_reference(b.index, new);
        }
    }
    if changed {
        edit.dirty = true;
    }
    rebuild
}

/// The anchors of `models` (the asset's own, or a clip's model and its
/// references'), the fields of the selected one, and buttons to add and
/// remove them.
pub(super) fn anchors_section(
    state: &Shared,
    edit: &AssetEdit,
    width: f32,
    value_w: f32,
) -> (Section, Option<AnchorBindings>) {
    let mut models: Vec<AssetId> = Vec::new();
    let clip_model = edit.clip_model();
    match (&edit.clip, clip_model) {
        (Some(clip), Some(clip_model)) => {
            models.push(clip_model);
            for reference in &clip.file.references {
                if !models.contains(&reference.model) {
                    models.push(reference.model);
                }
            }
        }
        (Some(_), None) => {}
        (None, _) => models.push(edit.meta.id),
    }
    let mut section = Section::new("Anchors", Some(icons::AXIS_3D), width);
    for &model in &models {
        let anchors = edit.anchors(model);
        let name = if model == edit.meta.id { "This model".to_string() } else { asset_name(model) };
        section.add_child(muted(name));
        for anchor in &anchors {
            let pick = AnchorPick { asset: model, name: anchor.name.clone() };
            let selected = edit.anchor.as_ref() == Some(&pick);
            let select_state = state.clone();
            let picked = pick.clone();
            let mut button = Button::new(
                Position::default(),
                Size::new(width - 16.0, 22.0),
                Box::new(move |_| {
                    with_edit(&select_state, |edit| {
                        if let Some(clip) = edit.clip.as_mut() {
                            clip.select(None);
                        }
                        edit.select_anchor(Some(picked.clone()));
                    })
                }),
            );
            button.set_style(ButtonStyle::Ghost);
            let on = anchor.bone.as_deref().map(base_name).unwrap_or("model");
            button.set_label(&format!("{}  ({on})", anchor.name));
            button.set_active(selected);
            section.add_child(Box::new(button));
        }
        // A new anchor on a clip's model goes on the selected bone.
        let bone = (Some(model) == clip_model)
            .then(|| edit.clip.as_ref().and_then(|clip| clip.selected.clone()))
            .flatten();
        let label = match &bone {
            Some(bone) => format!("Add on {}", base_name(bone)),
            None => "Add anchor".to_string(),
        };
        let add_state = state.clone();
        let mut add = small_button(
            &label,
            icons::PLUS,
            ButtonStyle::Secondary,
            Box::new(move |_| {
                let bone = bone.clone();
                with_edit(&add_state, |edit| {
                    if let Some(clip) = edit.clip.as_mut() {
                        clip.select(None);
                    }
                    edit.add_anchor(model, bone, [0.0; 3]);
                })
            }),
        );
        add.set_tooltip(if Some(model) == clip_model {
            "Add an anchor on the selected bone (on the model without one)"
        } else {
            "Add an anchor at this model's origin"
        });
        section.add_child(Box::new(add));
    }

    let Some(pick) = edit.anchor.clone().filter(|pick| models.contains(&pick.asset)) else {
        section.add_child(muted(
            "Click an anchor's diamond or a reference's model in the preview, or pick one here. Then drag its arrows or diamond to move it (Shift+drag a selected model anywhere), its rings to turn it.",
        ));
        return (section, None);
    };
    let Some(anchor) = edit.anchor_named(pick.asset, &pick.name) else {
        return (section, None);
    };
    let values = [
        anchor.position[0],
        anchor.position[1],
        anchor.position[2],
        anchor.rotation[0],
        anchor.rotation[1],
        anchor.rotation[2],
    ]
    .map(|v| DataSource::new(rounded(v)));
    let mut b = AnchorBindings {
        pick: pick.clone(),
        name: DataSource::new(anchor.name.clone()),
        bone: DataSource::new(anchor.bone.clone().unwrap_or_default()),
        values,
        seen: ([0.0; 6], String::new(), String::new()),
    };
    b.seen = b.now();
    section.add_child(Box::new(FieldRow::new(
        "Name",
        vec![UI::input_sized(b.name.clone(), Size::new(value_w, 24.0))],
    )));
    section.add_child(Box::new(FieldRow::new(
        "Bone",
        vec![UI::input_sized(b.bone.clone(), Size::new(value_w, 24.0))],
    )));
    let each = ((value_w - 8.0) / 3.0).max(30.0);
    let fields = |range: std::ops::Range<usize>| -> Vec<Box<dyn UIElement>> {
        range
            .map(|i| -> Box<dyn UIElement> { UI::input_sized(b.values[i].clone(), Size::new(each, 24.0)) })
            .collect()
    };
    section.add_child(Box::new(FieldRow::new("Position (m)", fields(0..3))));
    section.add_child(Box::new(FieldRow::new("Rotation (°)", fields(3..6))));
    let remove_state = state.clone();
    let mut remove = small_button(
        "Remove anchor",
        icons::TRASH,
        ButtonStyle::Danger,
        Box::new(move |_| with_edit(&remove_state, |edit| edit.remove_anchor())),
    );
    remove.set_tooltip("Remove the selected anchor from its model");
    section.add_child(Box::new(remove));
    if pick.asset != edit.meta.id {
        section.add_child(muted(format!(
            "Belongs to {}: saved with the clip.",
            asset_name(pick.asset)
        )));
    }
    (section, Some(b))
}

/// Carry what was typed into the selected anchor's fields over to it;
/// whether it was renamed (the lists show the new name once the typing
/// has stopped).
pub(super) fn sync_anchor(edit: &mut AssetEdit, b: &mut AnchorBindings) -> bool {
    let now = b.now();
    if now == b.seen {
        return false;
    }
    let seen = std::mem::replace(&mut b.seen, now.clone());
    let Some(mut anchor) = edit.anchor_named(b.pick.asset, &b.pick.name) else { return false };
    let (values, name, bone) = now;
    for i in 0..6 {
        if values[i] != seen.0[i] && values[i].is_finite() {
            if i < 3 {
                anchor.position[i] = values[i];
            } else {
                anchor.rotation[i - 3] = values[i];
            }
        }
    }
    if bone != seen.2 {
        let bone = bone.trim();
        anchor.bone = (!bone.is_empty()).then(|| bone.to_string());
    }
    let name = name.trim().to_string();
    let renamed = name != seen.1.trim() && !name.is_empty();
    if renamed && !edit.anchors(b.pick.asset).iter().any(|a| a.name == name) {
        anchor.name = name.clone();
    }
    let old_name = b.pick.name.clone();
    let new_name = anchor.name.clone();
    edit.set_anchor(b.pick.asset, &old_name, anchor);
    let renamed = new_name != old_name;
    b.pick.name = new_name;
    renamed
}
