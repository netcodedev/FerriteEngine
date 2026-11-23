//! The asset settings in the left column of the asset workspace.

use cgmath::{InnerSpace, Vector3};
use ferrite::core::{
    assets::{self, AssetId, AssetKind},
    model::grip::Grip,
    primitives::{Offset, Position, Size},
    renderer::text::icons,
    scene::Scene,
    utils::DataSource,
};
use ferrite_ui::ui::{
    element_handle::UIElementHandle,
    elements::{
        button::{Button, ButtonStyle},
        dropdown::Dropdown,
        label::Label,
        panel::Panel,
        scroll::ScrollArea,
        section::Section,
    },
    theme, UIElement, UI,
};
use glfw::{Glfw, WindowEvent};

use crate::editor::{
    drag::{DropAction, DropTarget},
    field_row::{FieldRow, LABEL_COLUMN},
    layout::Rect,
    send, Command, Shared,
};

use super::*;

pub(super) fn header_button(
    label: &str,
    icon: &str,
    style: ButtonStyle,
    on_click: Box<dyn Fn(&mut Scene)>,
) -> Button {
    let mut button = Button::new(Position::default(), Size::new(10.0, 26.0), on_click);
    button.set_style(style);
    button.set_icon(icon);
    button.set_label(label);
    button.fit(26.0);
    button
}

/// Name of a texture asset for a slot row.
fn material_name(material: Option<AssetId>) -> String {
    match material {
        None => tr!("asset_editor.inspector.from_file").into(),
        Some(id) => assets::database()
            .get(id)
            .map(|entry| entry.meta.name.clone())
            .unwrap_or_else(|| tr!("asset_editor.inspector.missing", id = id)),
    }
}

/// Widgets bound to the working copy, compared against it every frame.
struct Bindings {
    name: DataSource<String>,
    scale: DataSource<f32>,
    variant: DataSource<u32>,
    /// 0 is "automatic", then the LODs.
    lod: DataSource<u32>,
    /// Length of a clip in seconds.
    length: Option<DataSource<f32>>,
    /// How opaque a clip's model is drawn.
    model_opacity: Option<DataSource<f32>>,
    grip: Option<GripBindings>,
    references: Vec<anchors_ui::RefBindings>,
    anchor: Option<anchors_ui::AnchorBindings>,
}

/// The grip's widgets, with the values they were last seen at: only what
/// is typed is written back, so what the preview's handles set is kept.
struct GripBindings {
    held: DataSource<bool>,
    holding: DataSource<bool>,
    pose: DataSource<u32>,
    head: DataSource<u32>,
    edge: DataSource<u32>,
    roll: DataSource<f32>,
    point: [DataSource<f32>; 3],
    both: DataSource<bool>,
    other: [DataSource<f32>; 3],
    seen: GripValues,
}

#[derive(Clone, Copy, PartialEq)]
struct GripValues {
    head: u32,
    edge: u32,
    roll: f32,
    point: [f32; 3],
    other: [f32; 3],
}

impl GripBindings {
    fn values(&self) -> GripValues {
        GripValues {
            head: self.head.read(),
            edge: self.edge.read(),
            roll: self.roll.read(),
            point: [0, 1, 2].map(|i| self.point[i].read()),
            other: [0, 1, 2].map(|i| self.other[i].read()),
        }
    }
}

/// Three number fields in a row, for x, y and z.
fn xyz_fields(values: &[DataSource<f32>; 3], width: f32) -> Vec<Box<dyn UIElement>> {
    let each = ((width - 8.0) / 3.0).max(30.0);
    values
        .iter()
        .map(|value| -> Box<dyn UIElement> { UI::input_sized(value.clone(), Size::new(each, 24.0)) })
        .collect()
}

pub(super) fn rounded(v: f32) -> f32 {
    (v * 1000.0).round() / 1000.0
}

/// Left column of the asset workspace: the asset's settings.
pub struct AssetInspector {
    handle: UIElementHandle,
    state: Shared,
    panel: Panel,
    body: ScrollArea,
    save: Button,
    revert: Button,
    back: Button,
    bindings: Option<Bindings>,
    /// Rows a material asset can be dropped on.
    slot_rows: Vec<(UIElementHandle, usize)>,
    /// `(asset, structure counter, width, clip selection)` the panel was
    /// built for.
    built_for: Option<(AssetId, u64, i32, u64)>,
    /// When an anchor was last renamed by typing: the lists show the new
    /// name once the typing has stopped a while.
    renamed_at: Option<std::time::Instant>,
}

const ACTIONS_HEIGHT: f32 = 34.0;

impl AssetInspector {
    pub fn new(state: Shared) -> Self {
        let mut panel = Panel::new(tr!("asset_editor.inspector.title").into(), Position::default(), Size::new(330.0, 600.0));
        panel.set_movable(false);
        panel.set_fixed_size(true);
        panel.set_flat(true);
        let mut body = ScrollArea::new(Position::default(), Size::new(320.0, 500.0));
        body.set_gap(4.0);

        let save_state = state.clone();
        let mut save = header_button(
            tr!("common.save"),
            icons::SAVE,
            ButtonStyle::Primary,
            Box::new(move |_| send(&save_state, Command::SaveAsset)),
        );
        save.set_tooltip(tr!("asset_editor.inspector.save_tooltip"));
        let revert_state = state.clone();
        let mut revert = header_button(
            tr!("common.revert"),
            icons::ROTATE_CW,
            ButtonStyle::Secondary,
            Box::new(move |_| send(&revert_state, Command::RevertAsset)),
        );
        revert.set_tooltip(tr!("asset_editor.inspector.revert_tooltip"));
        let back_state = state.clone();
        let back = header_button(
            tr!("asset_editor.inspector.back"),
            icons::ARROW_LEFT,
            ButtonStyle::Ghost,
            Box::new(move |_| send(&back_state, Command::CloseAssetEditor)),
        );

        Self {
            handle: UIElementHandle::new(),
            state,
            panel,
            body,
            save,
            revert,
            back,
            bindings: None,
            slot_rows: Vec::new(),
            built_for: None,
            renamed_at: None,
        }
    }

    /// The whole left column.
    fn rect(&self) -> Rect {
        let layout = self.state.borrow().layout;
        Rect {
            x: layout.hierarchy.x,
            y: layout.hierarchy.y,
            w: layout.hierarchy.w,
            h: layout.hierarchy.h + layout.inspector.h,
        }
    }

    fn rebuild(&mut self, width: f32) {
        self.body.clear();
        self.slot_rows.clear();
        self.bindings = None;
        let state = self.state.borrow();
        let Some(edit) = &state.asset_edit else {
            return;
        };
        let value_w = width - LABEL_COLUMN - 16.0;
        let muted = |text: String| -> Box<dyn UIElement> {
            Box::new(Label::muted(text, theme::FONT_SIZE_SMALL + 0.5))
        };

        // What it is.
        let name = DataSource::new(edit.meta.name.clone());
        let mut general = Section::new(tr!("asset_editor.inspector.section_asset"), Some(icons::PACKAGE), width);
        general.add_child(Box::new(FieldRow::new(
            tr!("common.name"),
            vec![UI::input_sized(name.clone(), Size::new(value_w, 24.0))],
        )));
        general.add_child(Box::new(FieldRow::new(
            tr!("asset_editor.inspector.kind"),
            vec![muted(super::super::asset_browser::kind_label(&edit.meta.kind).to_string())],
        )));
        general.add_child(Box::new(FieldRow::new(
            tr!("asset_editor.inspector.file"),
            vec![muted(edit.source_path().to_string_lossy().replace('\\', "/"))],
        )));
        self.body.add_child(Box::new(general));

        let mut bindings = Bindings {
            name,
            scale: DataSource::new(1.0),
            variant: DataSource::new(edit.variant as u32),
            lod: DataSource::new(edit.lod.map_or(0, |lod| lod as u32 + 1)),
            length: None,
            model_opacity: None,
            grip: None,
            references: Vec::new(),
            anchor: None,
        };

        if let Some(clip) = &edit.clip {
            let length = DataSource::new(clip.duration());
            let mut section = Section::new(tr!("asset_editor.inspector.section_clip"), Some(icons::ACTIVITY), width);
            section.add_child(Box::new(FieldRow::new(
                tr!("asset_editor.inspector.length"),
                vec![UI::input_sized(length.clone(), Size::new(value_w.min(140.0), 24.0))],
            )));
            let model = clip
                .file
                .model
                .and_then(|id| assets::database().get(id).map(|e| e.meta.name.clone()))
                .unwrap_or_else(|| tr!("asset_editor.inspector.player_model").into());
            section.add_child(Box::new(FieldRow::new(tr!("asset_editor.inspector.model"), vec![muted(model)])));
            let opacity = DataSource::new(clip.file.model_opacity);
            section.add_child(Box::new(FieldRow::new(
                tr!("asset_editor.inspector.opacity"),
                vec![UI::slider_sized(opacity.clone(), 0.0, 1.0, Size::new(value_w, 24.0))],
            )));
            bindings.model_opacity = Some(opacity);
            let selected = clip
                .selected
                .as_deref()
                .map(|bone| ferrite::core::model::ik::base_name(bone).to_string())
                .unwrap_or_else(|| tr!("asset_editor.inspector.no_bone").into());
            section.add_child(Box::new(FieldRow::new(tr!("asset_editor.inspector.bone"), vec![muted(selected)])));
            section.add_child(muted(tr!("asset_editor.inspector.clip_keys").to_string()));
            self.body.add_child(Box::new(section));
            bindings.length = Some(length);

            if let Some((section, refs)) =
                anchors_ui::references_section(&self.state, edit, width, value_w)
            {
                self.body.add_child(Box::new(section));
                bindings.references = refs;
            }
            let (section, anchor) = anchors_ui::anchors_section(&self.state, edit, width, value_w);
            self.body.add_child(Box::new(section));
            bindings.anchor = anchor;

            let mut bones = Section::new(tr!("asset_editor.inspector.section_bones"), Some(icons::LIST_TREE), width);
            for bone in &clip.bones {
                let keyed = clip.file.track(bone).is_some();
                let state = self.state.clone();
                let name = bone.clone();
                let mut button = Button::new(
                    Position::default(),
                    Size::new(width - 16.0, 22.0),
                    Box::new(move |_| {
                        if let Some(clip) = state
                            .borrow_mut()
                            .asset_edit
                            .as_mut()
                            .and_then(|edit| edit.clip.as_mut())
                        {
                            clip.select(Some(name.clone()));
                        }
                    }),
                );
                button.set_style(ButtonStyle::Ghost);
                let label = ferrite::core::model::ik::base_name(bone);
                button.set_label(&if keyed { format!("{label}  \u{25c6}") } else { label.to_string() });
                button.set_active(clip.selected.as_deref() == Some(bone.as_str()));
                bones.add_child(Box::new(button));
            }
            self.body.add_child(Box::new(bones));
        }

        if let AssetKind::Model(model) = &edit.meta.kind {
            if !model.clips.is_empty() {
                let id = edit.meta.id;
                let state = self.state.clone();
                let mut section = Section::new(tr!("asset_editor.inspector.section_animation"), Some(icons::ACTIVITY), width);
                let mut button = header_button(
                    tr!("asset_editor.inspector.new_clip"),
                    icons::PLUS,
                    ButtonStyle::Primary,
                    Box::new(move |_| send(&state, Command::NewClipDialog(id))),
                );
                button.set_tooltip(tr!("asset_editor.inspector.new_clip_tooltip"));
                section.add_child(Box::new(button));
                self.body.add_child(Box::new(section));
            }
        }

        if let AssetKind::Model(model) = &edit.meta.kind {
            bindings.scale.write(model.scale);
            let mut settings = Section::new(tr!("asset_editor.inspector.section_model"), Some(icons::BOX), width);
            settings.add_child(Box::new(FieldRow::new(
                tr!("asset_editor.inspector.scale"),
                vec![UI::input_sized(
                    bindings.scale.clone(),
                    Size::new(value_w.min(140.0), 24.0),
                )],
            )));
            self.body.add_child(Box::new(settings));

            // How it is held, for models that are no characters.
            if model.clips.is_empty() {
                let held = DataSource::new(model.grip.is_some());
                let mut section = Section::new(tr!("asset_editor.inspector.section_grip"), Some(icons::POINTER), width);
                section.add_child(Box::new(FieldRow::new(
                    tr!("asset_editor.inspector.can_be_held"),
                    vec![UI::checkbox(held.clone()), muted(tr!("asset_editor.inspector.in_hand").to_string())],
                )));
                let grip = model.grip.clone().unwrap_or_else(|| Grip::at([0.0; 3]));
                let axes: Vec<String> = GRIP_AXES.iter().map(|(_, name)| name.to_string()).collect();
                let point = grip.point.map(|v| DataSource::new(rounded(v)));
                let other_at = grip.other_hand.unwrap_or(grip.point);
                let other = other_at.map(|v| DataSource::new(rounded(v)));
                let bindings_grip = GripBindings {
                    held: held.clone(),
                    holding: DataSource::new(edit.holding),
                    pose: DataSource::new(edit.hold_pose as u32),
                    head: DataSource::new(grip_axis(grip.along) as u32),
                    edge: DataSource::new(grip_axis(grip.facing) as u32),
                    roll: DataSource::new(rounded(grip.roll)),
                    point,
                    both: DataSource::new(grip.other_hand.is_some()),
                    other,
                    seen: GripValues {
                        head: 0,
                        edge: 0,
                        roll: 0.0,
                        point: [0.0; 3],
                        other: [0.0; 3],
                    },
                };
                if model.grip.is_some() {
                    let mut poses = vec![tr!("asset_editor.inspector.rest_pose").to_string()];
                    poses.extend(edit.hold_poses.iter().map(|pose| pose.label.clone()));
                    section.add_child(Box::new(FieldRow::new(
                        tr!("asset_editor.inspector.show_in_hand"),
                        vec![UI::checkbox(bindings_grip.holding.clone())],
                    )));
                    section.add_child(Box::new(FieldRow::new(
                        tr!("asset_editor.inspector.pose"),
                        vec![Box::new(Dropdown::new(
                            Position::default(),
                            Size::new(value_w, 24.0),
                            poses,
                            bindings_grip.pose.clone(),
                        ))],
                    )));
                    section.add_child(Box::new(FieldRow::new(
                        tr!("asset_editor.inspector.head_towards"),
                        vec![Box::new(Dropdown::new(
                            Position::default(),
                            Size::new(value_w.min(90.0), 24.0),
                            axes.clone(),
                            bindings_grip.head.clone(),
                        ))],
                    )));
                    section.add_child(Box::new(FieldRow::new(
                        tr!("asset_editor.inspector.edge_towards"),
                        vec![Box::new(Dropdown::new(
                            Position::default(),
                            Size::new(value_w.min(90.0), 24.0),
                            axes,
                            bindings_grip.edge.clone(),
                        ))],
                    )));
                    section.add_child(Box::new(FieldRow::new(
                        tr!("asset_editor.inspector.roll"),
                        vec![UI::input_sized(
                            bindings_grip.roll.clone(),
                            Size::new(value_w.min(90.0), 24.0),
                        )],
                    )));
                    section.add_child(Box::new(FieldRow::new(
                        tr!("asset_editor.inspector.hand_at"),
                        xyz_fields(&bindings_grip.point, value_w),
                    )));
                    section.add_child(Box::new(FieldRow::new(
                        tr!("asset_editor.inspector.both_hands"),
                        vec![UI::checkbox(bindings_grip.both.clone())],
                    )));
                    if grip.other_hand.is_some() {
                        section.add_child(Box::new(FieldRow::new(
                            tr!("asset_editor.inspector.other_hand_at"),
                            xyz_fields(&bindings_grip.other, value_w),
                        )));
                    }
                    section.add_child(muted(tr!("asset_editor.inspector.grip_hint").to_string()));
                }
                let mut bindings_grip = bindings_grip;
                bindings_grip.seen = bindings_grip.values();
                bindings.grip = Some(bindings_grip);
                self.body.add_child(Box::new(section));
            }

            // Where other models go on it.
            let (section, anchor) = anchors_ui::anchors_section(&self.state, edit, width, value_w);
            self.body.add_child(Box::new(section));
            bindings.anchor = anchor;

            // What the preview shows.
            let mut preview = Section::new(tr!("asset_editor.inspector.section_preview"), Some(icons::EYE), width);
            let variants: Vec<String> = model.variants.iter().map(|v| v.name.clone()).collect();
            let mut lods = vec![tr!("asset_editor.inspector.lod_auto").to_string()];
            if let Some(variant) = model.variants.get(edit.variant) {
                lods.extend(variant.lods.iter().cloned());
            }
            preview.add_child(Box::new(FieldRow::new(
                tr!("asset_editor.inspector.variant"),
                vec![Box::new(Dropdown::new(
                    Position::default(),
                    Size::new(value_w, 24.0),
                    variants,
                    bindings.variant.clone(),
                ))],
            )));
            preview.add_child(Box::new(FieldRow::new(
                tr!("asset_editor.inspector.lod"),
                vec![Box::new(Dropdown::new(
                    Position::default(),
                    Size::new(value_w, 24.0),
                    lods,
                    bindings.lod.clone(),
                ))],
            )));
            self.body.add_child(Box::new(preview));

            // One section per material.
            for (index, material) in model.materials.iter().enumerate() {
                // A material no mesh of the file uses (Assimp's default one).
                if material.unused && material.material.is_none() {
                    continue;
                }
                let title = if material.name.is_empty() {
                    tr!("asset_editor.inspector.material_n", n = index + 1)
                } else {
                    tr!("asset_editor.inspector.material_named", name = material.name)
                };
                let mut section = Section::new(&title, Some(icons::LAYERS), width);
                let pick_state = self.state.clone();
                let mut pick = Button::new(
                    Position::default(),
                    Size::new(24.0, 24.0),
                    Box::new(move |_| send(&pick_state, Command::PickModelMaterial { material: index })),
                );
                pick.set_style(ButtonStyle::Secondary);
                pick.set_icon(icons::LAYOUT_GRID);
                pick.set_tooltip(tr!("asset_editor.inspector.pick_material_tooltip"));
                let mut row_widgets: Vec<Box<dyn UIElement>> = vec![Box::new(pick)];
                if let Some(id) = material.material {
                    let open_state = self.state.clone();
                    let mut open = Button::new(
                        Position::default(),
                        Size::new(24.0, 24.0),
                        Box::new(move |_| send(&open_state, Command::EditAsset { id, variant: None })),
                    );
                    open.set_style(ButtonStyle::Secondary);
                    open.set_icon(icons::PENCIL);
                    open.set_tooltip(tr!("asset_editor.inspector.edit_material_tooltip"));
                    row_widgets.push(Box::new(open));
                }
                row_widgets.push(muted(material_name(material.material)));
                let row = Box::new(FieldRow::new(tr!("asset_editor.inspector.material"), row_widgets));
                self.slot_rows.push((*row.get_handle(), index));
                section.add_child(row);
                self.body.add_child(Box::new(section));
            }
        }
        // The clips of an animation file, or those a model brings along.
        let clips = edit.meta.clips();
        if !clips.is_empty() && edit.clip.is_none() {
            let mut section = Section::new(tr!("asset_editor.inspector.section_clips"), Some(icons::ACTIVITY), width);
            for clip in clips {
                section.add_child(Box::new(FieldRow::new(
                    &clip.name,
                    vec![muted(tr!("asset_editor.inspector.seconds", value = format!("{:.2}", clip.duration)))],
                )));
            }
            self.body.add_child(Box::new(section));
        }
        self.bindings = Some(bindings);
    }

    /// Carry widget edits over to the working copy.
    fn sync(&mut self) {
        let Some(bindings) = &self.bindings else {
            return;
        };
        let mut state = self.state.borrow_mut();
        let Some(edit) = state.asset_edit.as_mut() else {
            return;
        };
        let name = bindings.name.read();
        if name != edit.meta.name {
            edit.meta.name = name;
            edit.dirty = true;
        }
        let variant = bindings.variant.read() as usize;
        if variant != edit.variant {
            edit.variant = variant;
            // The LOD list belongs to the variant.
            edit.lod = None;
            edit.structure += 1;
            edit.preview += 1;
        }
        let lod = bindings.lod.read().checked_sub(1).map(|lod| lod as usize);
        if lod != edit.lod {
            edit.lod = lod;
        }
        let mut reload = false;
        let mut dirty = false;
        if let Some(model) = edit.meta.as_model_mut() {
            let scale = bindings.scale.read();
            // A half-typed number must not shrink the model to nothing.
            if scale > 0.0 && (scale - model.scale).abs() > 1e-6 {
                model.scale = scale;
                dirty = true;
                reload = true;
            }
        }
        if let (Some(opacity), Some(clip)) = (&bindings.model_opacity, edit.clip.as_mut()) {
            let opacity = opacity.read().clamp(0.0, 1.0);
            if (opacity - clip.file.model_opacity).abs() > 1e-4 {
                clip.file.model_opacity = opacity;
                dirty = true;
            }
        }
        if let (Some(length), Some(clip)) = (&bindings.length, edit.clip.as_mut()) {
            let length = length.read();
            // A half-typed number must not cut the clip short.
            if length >= 0.05 && (length - clip.duration()).abs() > 1e-4 {
                clip.set_duration(length);
                dirty = true;
            }
        }
        if dirty {
            edit.dirty = true;
        }
        if reload {
            edit.preview += 1;
        }
    }

    /// Carry what was typed or picked in the grip section over to the
    /// working copy.
    fn sync_grip(&mut self) {
        let Some(b) = self.bindings.as_mut().and_then(|b| b.grip.as_mut()) else {
            return;
        };
        let mut state = self.state.borrow_mut();
        let Some(edit) = state.asset_edit.as_mut() else {
            return;
        };
        let held = b.held.read();
        if held != edit.grip().is_some() {
            edit.set_grip(held.then(|| Grip::at([0.0; 3])));
            if !held && edit.holding {
                edit.holding = false;
                edit.preview += 1;
            }
            edit.structure += 1;
            return;
        }
        let Some(mut grip) = edit.grip().cloned() else {
            return;
        };
        let holding = b.holding.read();
        if holding != edit.holding {
            edit.holding = holding;
            edit.preview += 1;
        }
        edit.hold_pose = b.pose.read() as usize;

        let now = b.values();
        let seen = b.seen;
        b.seen = now;
        let mut rebuild = false;
        if now.head != seen.head {
            grip.along = GRIP_AXES[now.head as usize % GRIP_AXES.len()].0;
        }
        if now.edge != seen.edge {
            grip.facing = GRIP_AXES[now.edge as usize % GRIP_AXES.len()].0;
        }
        // The edge has to be square to the handle.
        let along = Vector3::from(grip.along);
        if Vector3::from(grip.facing).dot(along).abs() > 0.5 {
            if let Some((axis, _)) = GRIP_AXES
                .iter()
                .find(|(axis, _)| Vector3::from(*axis).dot(along).abs() < 0.5)
            {
                grip.facing = *axis;
                rebuild = true;
            }
        }
        if now.roll != seen.roll {
            grip.roll = now.roll;
        }
        for i in 0..3 {
            if now.point[i] != seen.point[i] {
                grip.point[i] = now.point[i];
            }
        }
        let both = b.both.read();
        if both != grip.other_hand.is_some() {
            // A hand's width and a bit further from the head.
            grip.other_hand = both.then(|| {
                let p = Vector3::from(grip.point) - along.normalize() * 0.22;
                [p.x, p.y, p.z]
            });
            rebuild = true;
        } else if let Some(other) = &mut grip.other_hand {
            for i in 0..3 {
                if now.other[i] != seen.other[i] {
                    other[i] = now.other[i];
                }
            }
        }
        edit.set_grip(Some(grip));
        if rebuild {
            edit.structure += 1;
        }
    }

    /// Carry what was picked for the references and typed for the
    /// selected anchor over to the working copies.
    fn sync_anchors(&mut self) {
        let Some(b) = self.bindings.as_mut() else { return };
        let mut state = self.state.borrow_mut();
        let Some(edit) = state.asset_edit.as_mut() else { return };
        let mut rebuild = anchors_ui::sync_references(edit, &mut b.references);
        if let Some(anchor) = b.anchor.as_mut() {
            if anchors_ui::sync_anchor(edit, anchor) {
                self.renamed_at = Some(std::time::Instant::now());
            }
        }
        if self
            .renamed_at
            .is_some_and(|at| at.elapsed().as_secs_f32() > 1.5)
        {
            self.renamed_at = None;
            rebuild = true;
        }
        if rebuild {
            edit.structure += 1;
        }
    }

    /// Texture slots take texture assets dragged out of the browser.
    fn register_drop_targets(&self) {
        let panel = self.rect();
        let mut targets = Vec::new();
        for (handle, material) in &self.slot_rows {
            let Some(row) = self.body.get_child(handle) else {
                continue;
            };
            let rect = Rect {
                x: panel.x + 4.0,
                y: row.get_offset().y - 1.0,
                w: (panel.w - 8.0).max(1.0),
                h: row.get_size().height + 2.0,
            };
            if rect.y < panel.y || rect.y + rect.h > panel.y + panel.h {
                continue;
            }
            targets.push(DropTarget {
                rect,
                action: DropAction::ModelMaterial { material: *material },
            });
        }
        self.state.borrow_mut().drop_targets.extend(targets);
    }

    fn buttons_mut(&mut self) -> [&mut Button; 3] {
        [&mut self.save, &mut self.revert, &mut self.back]
    }
}

impl UIElement for AssetInspector {
    fn update(&mut self, scene: &mut Scene) {
        let rect = self.rect();
        self.panel.set_size(Size::new(rect.w, rect.h));
        self.panel.set_position(Position::new(rect.x, rect.y, 0.0));

        let (key, dirty) = {
            let state = self.state.borrow();
            match &state.asset_edit {
                Some(edit) => (
                    Some((
                        edit.meta.id,
                        edit.structure,
                        rect.w as i32,
                        edit.clip.as_ref().map_or(0, |clip| clip.selection),
                    )),
                    edit.dirty,
                ),
                None => (None, false),
            }
        };

        let header = self.panel.header_height();
        let ay = rect.y + header + 4.0;
        let mut x = rect.x + 6.0;
        self.save.set_enabled(dirty);
        self.revert.set_enabled(dirty);
        for button in [&mut self.save, &mut self.revert] {
            button.set_offset(Offset { x, y: ay });
            x += button.get_size().width + 4.0;
        }
        let back_x = (rect.x + rect.w - 6.0 - self.back.get_size().width).max(x);
        self.back.set_offset(Offset { x: back_x, y: ay });

        let body_top = rect.y + header + ACTIONS_HEIGHT + 4.0;
        self.body
            .set_position(Position::new(rect.x + 4.0, body_top, 0.0));
        self.body.set_size(Size::new(
            (rect.w - 8.0).max(10.0),
            (rect.y + rect.h - body_top - 4.0).max(10.0),
        ));

        if key != self.built_for {
            self.rebuild((rect.w - 24.0).max(200.0));
            self.body.set_z_index(2.0);
            self.built_for = key;
        }
        self.sync();
        self.sync_grip();
        self.sync_anchors();

        self.panel.update(scene);
        for button in self.buttons_mut() {
            button.update(scene);
        }
        self.body.update(scene);
        self.register_drop_targets();
    }

    fn render(&self) {
        self.panel.render();
        self.body.render();
        self.save.render();
        self.revert.render();
        self.back.render();
    }

    fn render_overlay(&self) {
        self.body.render_overlay();
        self.save.render_overlay();
        self.revert.render_overlay();
        self.back.render_overlay();
    }

    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut glfw::Window,
        glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        for button in self.buttons_mut() {
            if button.handle_events(scene, window, glfw, event)
                && !matches!(event, WindowEvent::CursorPos(..))
            {
                return true;
            }
        }
        if self.body.handle_events(scene, window, glfw, event) {
            return true;
        }
        if let WindowEvent::MouseButton(..) | WindowEvent::Scroll(..) = event {
            let (x, y) = window.get_cursor_pos();
            return self.rect().contains(x as f32, y as f32);
        }
        false
    }

    fn add_child(&mut self, _child: Box<dyn UIElement>) {}
    fn add_child_to(&mut self, _parent: UIElementHandle, _child: Box<dyn UIElement>) {}
    fn contains_child(&self, _handle: &UIElementHandle) -> bool {
        false
    }
    fn get_child(&self, _handle: &UIElementHandle) -> Option<&Box<dyn UIElement>> {
        None
    }
    fn get_child_mut(&mut self, _handle: &UIElementHandle) -> Option<&mut Box<dyn UIElement>> {
        None
    }
    fn get_handle(&self) -> &UIElementHandle {
        &self.handle
    }
    fn get_offset(&self) -> &Offset {
        self.panel.get_offset()
    }
    fn set_offset(&mut self, _offset: Offset) {}
    fn get_size(&self) -> &Size {
        self.panel.get_size()
    }
    fn set_z_index(&mut self, z_index: f32) {
        self.panel.set_z_index(z_index);
        self.body.set_z_index(z_index + 2.0);
        for button in self.buttons_mut() {
            button.set_z_index(z_index + 3.0);
        }
    }
}
