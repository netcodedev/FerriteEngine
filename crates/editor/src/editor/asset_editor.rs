//! The asset workspace: settings of one asset in the left column and a
//! preview of it where the scene viewport normally is. Edits are made on a
//! working copy and reach the asset (and whatever uses it) when saved; the
//! preview always shows the working copy.

use std::{path::PathBuf, time::Instant};

use cgmath::{
    perspective, Deg, EuclideanSpace, InnerSpace, Matrix4, Point3, SquareMatrix, Transform,
    Vector3, Vector4,
};
use ferrite::{
    core::{
        assets::{self, AssetId, AssetKind, AssetMeta, ClipInfo, MaterialMap, ModelSettings},
        model::{
            clip_file::{is_clip_file, BonePose, Transform as BoneTransform},
            grip::{self, Grip, Hand},
            ik::{IkChain, Skeleton},
            Animation, Model, ModelBuilder,
        },
        primitives::{Offset, Position, Size},
        renderer::{
            line::{Line, LineRenderer},
            pbr::Lighting,
            text::{icons, Text},
        },
        scene::Scene,
        utils::DataSource,
    },
    terrain::scatter::{LodChoice, ScatterInstances, ScatterModel, ScatterSpec},
};
use ferrite_ui::ui::{
    element_handle::UIElementHandle,
    elements::{
        button::{Button, ButtonStyle},
        dropdown::Dropdown,
        image::Image,
        label::Label,
        panel::Panel,
        scroll::ScrollArea,
        section::Section,
    },
    theme, UIElement, UI,
};
use glfw::{Action, Glfw, MouseButton, WindowEvent};

use super::{
    clip_editor::{
        ring_points, ClipEdit, ClipRequest, Timeline, TimelineAction, TimelineView, AXES,
        RING_COLORS, TIMELINE_HEIGHT,
    },
    drag::{DropAction, DropTarget},
    field_row::{FieldRow, LABEL_COLUMN},
    layout::Rect,
    send, Command, Shared,
};

/// A texture a material can be given.
pub use ferrite::core::assets::MaterialMap as TextureSlot;

/// The asset open in the asset workspace.
pub struct AssetEdit {
    /// Working copy of the asset's sidecar.
    pub meta: AssetMeta,
    /// Folder of the asset's source file.
    pub dir: PathBuf,
    /// Whether the working copy differs from what is saved.
    pub dirty: bool,
    /// Variant shown in the preview (index into the model's variants).
    pub variant: usize,
    /// LOD shown in the preview; `None` picks it by camera distance.
    pub lod: Option<usize>,
    /// Goes up when the settings panel has to be rebuilt.
    pub structure: u64,
    /// Goes up when the preview has to load the model again.
    pub preview: u64,
    /// The clip, when the asset is a clip made in the editor.
    pub clip: Option<ClipEdit>,
    /// The preview shows the model held in the player model's hand (it
    /// has a grip).
    pub holding: bool,
    /// What the player model does while holding it: the rest pose, or one
    /// of these clips.
    pub hold_poses: Vec<HoldPose>,
    /// Index into `hold_poses` + 1; 0 is the rest pose.
    pub hold_pose: usize,
}

/// An animation clip to show a held model in.
#[derive(Clone, Debug, PartialEq)]
pub struct HoldPose {
    pub label: String,
    pub source: PathBuf,
    /// The clip's name in a file of several.
    pub clip: Option<String>,
}

/// Every animation clip of the project, to hold a model in.
fn hold_poses() -> Vec<HoldPose> {
    let database = assets::database();
    let mut poses: Vec<HoldPose> = database
        .iter()
        .filter(|entry| matches!(entry.meta.kind, AssetKind::Animation(_)))
        .flat_map(|entry| {
            let clips = entry.meta.clips();
            let several = clips.len() > 1;
            clips
                .iter()
                .map(|clip| HoldPose {
                    label: if several {
                        format!("{} / {}", entry.meta.name, clip.name)
                    } else {
                        entry.meta.name.clone()
                    },
                    source: entry.source_path(),
                    clip: several.then(|| clip.name.clone()),
                })
                .collect::<Vec<_>>()
        })
        .collect();
    poses.sort_by(|a, b| a.label.cmp(&b.label));
    poses
}

/// The six axis directions a grip's directions are picked from.
const GRIP_AXES: [([f32; 3], &str); 6] = [
    ([1.0, 0.0, 0.0], "+X"),
    ([-1.0, 0.0, 0.0], "-X"),
    ([0.0, 1.0, 0.0], "+Y"),
    ([0.0, -1.0, 0.0], "-Y"),
    ([0.0, 0.0, 1.0], "+Z"),
    ([0.0, 0.0, -1.0], "-Z"),
];

/// The axis direction closest to `v`.
fn grip_axis(v: [f32; 3]) -> usize {
    let v = Vector3::from(v);
    (0..GRIP_AXES.len())
        .max_by(|a, b| {
            let da = Vector3::from(GRIP_AXES[*a].0).dot(v);
            let db = Vector3::from(GRIP_AXES[*b].0).dot(v);
            da.total_cmp(&db)
        })
        .unwrap_or(0)
}

impl AssetEdit {
    /// Start editing an asset, showing the named variant if given.
    pub fn open(id: AssetId, variant: Option<&str>) -> Result<Self, String> {
        let database = assets::database();
        let entry = database
            .get(id)
            .ok_or_else(|| format!("asset {id} does not exist"))?;
        let variant = variant
            .and_then(|name| {
                entry
                    .meta
                    .as_model()?
                    .variants
                    .iter()
                    .position(|v| v.name == name)
            })
            .unwrap_or(0);
        let source = entry.source_path();
        let clip = if is_clip_file(&source) {
            Some(ClipEdit::load(&source)?)
        } else {
            None
        };
        let is_model = entry.meta.as_model().is_some();
        let meta = entry.meta.clone();
        let dir = entry.dir.clone();
        drop(database);
        Ok(Self {
            meta,
            dir,
            dirty: false,
            variant,
            lod: None,
            structure: 0,
            preview: 0,
            clip,
            holding: false,
            hold_poses: if is_model { hold_poses() } else { Vec::new() },
            hold_pose: 0,
        })
    }

    pub fn source_path(&self) -> PathBuf {
        self.dir.join(&self.meta.source)
    }

    /// The grip of the working copy, if it is a model that has one.
    pub fn grip(&self) -> Option<&Grip> {
        self.meta.as_model()?.grip.as_ref()
    }

    /// Change the grip of the working copy.
    pub fn set_grip(&mut self, grip: Option<Grip>) {
        let Some(model) = self.meta.as_model_mut() else {
            return;
        };
        if model.grip != grip {
            model.grip = grip;
            self.dirty = true;
        }
    }

    /// Give a material a texture (or none, for the automatic one).
    pub fn set_texture(&mut self, material: usize, slot: TextureSlot, texture: Option<AssetId>) {
        let Some(material) = self
            .meta
            .as_model_mut()
            .and_then(|model| model.materials.get_mut(material))
        else {
            return;
        };
        if material.map(slot) != texture {
            material.set_map(slot, texture);
            self.dirty = true;
            self.structure += 1;
            self.preview += 1;
        }
    }

    /// Write the working copy to the asset.
    pub fn save(&mut self) -> Result<(), String> {
        if let Some(clip) = &self.clip {
            clip.to_save()
                .save(&self.source_path())
                .map_err(|e| e.to_string())?;
            // The clip's length is listed with the asset; saving its
            // settings also makes graphs using it load it again.
            if let AssetKind::Animation(settings) = &mut self.meta.kind {
                let name = settings
                    .clips
                    .first()
                    .map(|c| c.name.clone())
                    .unwrap_or_else(|| self.meta.name.clone());
                settings.clips = vec![ClipInfo {
                    name,
                    duration: clip.duration(),
                }];
            }
        }
        assets::database()
            .update(self.meta.clone())
            .map_err(|e| e.to_string())?;
        self.dirty = false;
        Ok(())
    }

    /// Throw the edits away and go back to what is saved.
    pub fn revert(&mut self) -> Result<(), String> {
        let saved = assets::database()
            .get(self.meta.id)
            .map(|entry| entry.meta.clone())
            .ok_or_else(|| format!("asset {} does not exist any more", self.meta.id))?;
        self.meta = saved;
        if self.clip.is_some() {
            let mut clip = ClipEdit::load(&self.source_path())?;
            if let Some(old) = &self.clip {
                clip.selected = old.selected.clone();
                clip.rest = old.rest.clone();
                clip.bones = old.bones.clone();
                clip.keys = old.keys + 1;
                clip.set_time(old.time);
            }
            self.clip = Some(clip);
        }
        self.dirty = false;
        self.structure += 1;
        self.preview += 1;
        Ok(())
    }
}

fn header_button(
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
fn texture_name(texture: Option<AssetId>) -> String {
    match texture {
        None => "automatic".into(),
        Some(id) => assets::database()
            .get(id)
            .map(|entry| entry.meta.name.clone())
            .unwrap_or_else(|| format!("missing ({id})")),
    }
}

/// Widgets bound to the working copy, compared against it every frame.
struct Bindings {
    name: DataSource<String>,
    scale: DataSource<f32>,
    variant: DataSource<u32>,
    /// 0 is "automatic", then the LODs.
    lod: DataSource<u32>,
    foliage: Vec<DataSource<bool>>,
    /// Length of a clip in seconds.
    length: Option<DataSource<f32>>,
    grip: Option<GripBindings>,
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

fn rounded(v: f32) -> f32 {
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
    /// Rows a texture asset can be dropped on.
    slot_rows: Vec<(UIElementHandle, usize, TextureSlot)>,
    /// `(asset, structure counter, width, clip selection)` the panel was
    /// built for.
    built_for: Option<(AssetId, u64, i32, u64)>,
}

const ACTIONS_HEIGHT: f32 = 34.0;

impl AssetInspector {
    pub fn new(state: Shared) -> Self {
        let mut panel = Panel::new("Asset".into(), Position::default(), Size::new(330.0, 600.0));
        panel.set_movable(false);
        panel.set_fixed_size(true);
        panel.set_flat(true);
        let mut body = ScrollArea::new(Position::default(), Size::new(320.0, 500.0));
        body.set_gap(4.0);

        let save_state = state.clone();
        let mut save = header_button(
            "Save",
            icons::SAVE,
            ButtonStyle::Primary,
            Box::new(move |_| send(&save_state, Command::SaveAsset)),
        );
        save.set_tooltip("Save the asset (Ctrl+S)");
        let revert_state = state.clone();
        let mut revert = header_button(
            "Revert",
            icons::ROTATE_CW,
            ButtonStyle::Secondary,
            Box::new(move |_| send(&revert_state, Command::RevertAsset)),
        );
        revert.set_tooltip("Discard the unsaved changes");
        let back_state = state.clone();
        let back = header_button(
            "Back to scene",
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
        let mut general = Section::new("Asset", Some(icons::PACKAGE), width);
        general.add_child(Box::new(FieldRow::new(
            "Name",
            vec![UI::input_sized(name.clone(), Size::new(value_w, 24.0))],
        )));
        general.add_child(Box::new(FieldRow::new(
            "Kind",
            vec![muted(edit.meta.kind.label().to_string())],
        )));
        general.add_child(Box::new(FieldRow::new(
            "File",
            vec![muted(edit.source_path().to_string_lossy().replace('\\', "/"))],
        )));
        self.body.add_child(Box::new(general));

        let mut bindings = Bindings {
            name,
            scale: DataSource::new(1.0),
            variant: DataSource::new(edit.variant as u32),
            lod: DataSource::new(edit.lod.map_or(0, |lod| lod as u32 + 1)),
            foliage: Vec::new(),
            length: None,
            grip: None,
        };

        if let Some(clip) = &edit.clip {
            let length = DataSource::new(clip.duration());
            let mut section = Section::new("Clip", Some(icons::ACTIVITY), width);
            section.add_child(Box::new(FieldRow::new(
                "Length (s)",
                vec![UI::input_sized(length.clone(), Size::new(value_w.min(140.0), 24.0))],
            )));
            let model = clip
                .file
                .model
                .and_then(|id| assets::database().get(id).map(|e| e.meta.name.clone()))
                .unwrap_or_else(|| "player model".into());
            section.add_child(Box::new(FieldRow::new("Model", vec![muted(model)])));
            let selected = clip
                .selected
                .as_deref()
                .map(|bone| ferrite::core::model::ik::base_name(bone).to_string())
                .unwrap_or_else(|| "none (click a joint)".into());
            section.add_child(Box::new(FieldRow::new("Bone", vec![muted(selected)])));
            section.add_child(muted(
                "Space: play, K: key, Delete: remove key"
                    .to_string(),
            ));
            self.body.add_child(Box::new(section));
            bindings.length = Some(length);

            let mut bones = Section::new("Bones", Some(icons::LIST_TREE), width);
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
                let mut section = Section::new("Animation", Some(icons::ACTIVITY), width);
                let mut button = header_button(
                    "New clip",
                    icons::PLUS,
                    ButtonStyle::Primary,
                    Box::new(move |_| send(&state, Command::NewClipDialog(id))),
                );
                button.set_tooltip("Make a new animation clip posed on this model");
                section.add_child(Box::new(button));
                self.body.add_child(Box::new(section));
            }
        }

        if let AssetKind::Model(model) = &edit.meta.kind {
            bindings.scale.write(model.scale);
            let mut settings = Section::new("Model", Some(icons::BOX), width);
            settings.add_child(Box::new(FieldRow::new(
                "Scale",
                vec![UI::input_sized(
                    bindings.scale.clone(),
                    Size::new(value_w.min(140.0), 24.0),
                )],
            )));
            self.body.add_child(Box::new(settings));

            // How it is held, for models that are no characters.
            if model.clips.is_empty() {
                let held = DataSource::new(model.grip.is_some());
                let mut section = Section::new("Grip", Some(icons::POINTER), width);
                section.add_child(Box::new(FieldRow::new(
                    "Can be held",
                    vec![UI::checkbox(held.clone()), muted("in a character's hand".to_string())],
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
                    let mut poses = vec!["Rest pose".to_string()];
                    poses.extend(edit.hold_poses.iter().map(|pose| pose.label.clone()));
                    section.add_child(Box::new(FieldRow::new(
                        "Show in hand",
                        vec![UI::checkbox(bindings_grip.holding.clone())],
                    )));
                    section.add_child(Box::new(FieldRow::new(
                        "Pose",
                        vec![Box::new(Dropdown::new(
                            Position::default(),
                            Size::new(value_w, 24.0),
                            poses,
                            bindings_grip.pose.clone(),
                        ))],
                    )));
                    section.add_child(Box::new(FieldRow::new(
                        "Head towards",
                        vec![Box::new(Dropdown::new(
                            Position::default(),
                            Size::new(value_w.min(90.0), 24.0),
                            axes.clone(),
                            bindings_grip.head.clone(),
                        ))],
                    )));
                    section.add_child(Box::new(FieldRow::new(
                        "Edge towards",
                        vec![Box::new(Dropdown::new(
                            Position::default(),
                            Size::new(value_w.min(90.0), 24.0),
                            axes,
                            bindings_grip.edge.clone(),
                        ))],
                    )));
                    section.add_child(Box::new(FieldRow::new(
                        "Roll (°)",
                        vec![UI::input_sized(
                            bindings_grip.roll.clone(),
                            Size::new(value_w.min(90.0), 24.0),
                        )],
                    )));
                    section.add_child(Box::new(FieldRow::new(
                        "Hand at",
                        xyz_fields(&bindings_grip.point, value_w),
                    )));
                    section.add_child(Box::new(FieldRow::new(
                        "Both hands",
                        vec![UI::checkbox(bindings_grip.both.clone())],
                    )));
                    if grip.other_hand.is_some() {
                        section.add_child(Box::new(FieldRow::new(
                            "Other hand at",
                            xyz_fields(&bindings_grip.other, value_w),
                        )));
                    }
                    section.add_child(muted(
                        "In the preview: drag the model to move it in the hand, the blue diamond for the other hand."
                            .to_string(),
                    ));
                }
                let mut bindings_grip = bindings_grip;
                bindings_grip.seen = bindings_grip.values();
                bindings.grip = Some(bindings_grip);
                self.body.add_child(Box::new(section));
            }

            // What the preview shows.
            let mut preview = Section::new("Preview", Some(icons::EYE), width);
            let variants: Vec<String> = model.variants.iter().map(|v| v.name.clone()).collect();
            let mut lods = vec!["automatic".to_string()];
            if let Some(variant) = model.variants.get(edit.variant) {
                lods.extend(variant.lods.iter().cloned());
            }
            preview.add_child(Box::new(FieldRow::new(
                "Variant",
                vec![Box::new(Dropdown::new(
                    Position::default(),
                    Size::new(value_w, 24.0),
                    variants,
                    bindings.variant.clone(),
                ))],
            )));
            preview.add_child(Box::new(FieldRow::new(
                "LOD",
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
                let title = if material.name.is_empty() {
                    format!("Material {}", index + 1)
                } else {
                    format!("Material: {}", material.name)
                };
                let mut section = Section::new(&title, Some(icons::LAYERS), width);
                for slot in TextureSlot::ALL {
                    let pick_state = self.state.clone();
                    let mut pick = Button::new(
                        Position::default(),
                        Size::new(24.0, 24.0),
                        Box::new(move |_| {
                            send(
                                &pick_state,
                                Command::PickAssetTexture {
                                    material: index,
                                    slot,
                                },
                            )
                        }),
                    );
                    pick.set_style(ButtonStyle::Secondary);
                    pick.set_icon(icons::LAYOUT_GRID);
                    pick.set_tooltip("Choose a texture asset");
                    let clear_state = self.state.clone();
                    let mut clear = Button::new(
                        Position::default(),
                        Size::new(24.0, 24.0),
                        Box::new(move |_| {
                            send(
                                &clear_state,
                                Command::SetAssetTexture {
                                    material: index,
                                    slot,
                                    texture: None,
                                },
                            )
                        }),
                    );
                    clear.set_style(ButtonStyle::Danger);
                    clear.set_icon(icons::X);
                    clear.set_tooltip("Back to the automatic texture");
                    let row = Box::new(FieldRow::new(
                        slot.label(),
                        vec![
                            Box::new(pick),
                            Box::new(clear),
                            muted(texture_name(material.map(slot))),
                        ],
                    ));
                    self.slot_rows.push((*row.get_handle(), index, slot));
                    section.add_child(row);
                }
                let foliage = DataSource::new(material.foliage);
                section.add_child(Box::new(FieldRow::new(
                    "foliage",
                    vec![
                        UI::checkbox(foliage.clone()),
                        muted("no collision".to_string()),
                    ],
                )));
                bindings.foliage.push(foliage);
                self.body.add_child(Box::new(section));
            }
        }
        // The clips of an animation file, or those a model brings along.
        let clips = edit.meta.clips();
        if !clips.is_empty() && edit.clip.is_none() {
            let mut section = Section::new("Clips", Some(icons::ACTIVITY), width);
            for clip in clips {
                section.add_child(Box::new(FieldRow::new(
                    &clip.name,
                    vec![muted(format!("{:.2} s", clip.duration))],
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
            for (material, foliage) in model.materials.iter_mut().zip(&bindings.foliage) {
                if material.foliage != foliage.read() {
                    material.foliage = foliage.read();
                    dirty = true;
                }
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

    /// Texture slots take texture assets dragged out of the browser.
    fn register_drop_targets(&self) {
        let panel = self.rect();
        let mut targets = Vec::new();
        for (handle, material, slot) in &self.slot_rows {
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
                action: DropAction::MaterialTexture {
                    material: *material,
                    slot: *slot,
                },
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

/// Seconds the settings have to stay untouched before the preview loads
/// the model again (typing a scale would otherwise reload per digit).
const RELOAD_DELAY: f32 = 0.35;
/// Camera distance in model radii at which a model fits the view.
const DEFAULT_ZOOM: f32 = 3.6;
/// Longest edge (px) a texture is shown at, room permitting.
const TEXTURE_SIDE: f32 = 384.0;

struct Orbit {
    yaw: f32,
    pitch: f32,
    /// In multiples of the model's radius.
    zoom: f32,
    dragging: Option<(f64, f64)>,
}

/// What the preview currently shows.
enum Shown {
    Nothing,
    Model {
        instances: ScatterInstances,
        /// Centre and radius of the shown variant.
        center: Point3<f32>,
        radius: f32,
        floor: f32,
    },
    /// A model with bones, drawn skinned so its skeleton can be posed.
    Rigged(Box<Rigged>),
    /// A model held in the player model's hand, as its grip says.
    Held(Box<Held>),
    Texture(Image),
    /// Nothing to draw for this kind of asset; says so.
    Note(String),
    Failed(String),
}

/// A skinned model with the skeleton it is posed by.
struct Rigged {
    model: Model,
    skeleton: Skeleton,
    chains: Vec<IkChain>,
    center: Point3<f32>,
    radius: f32,
    floor: f32,
}

/// Load a model file as the working copy's settings say: its scale, the
/// textures given to its materials, the variant shown.
fn load_model(
    source: &std::path::Path,
    settings: &ModelSettings,
    variant: usize,
) -> Result<Model, String> {
    let overrides: Vec<Vec<(MaterialMap, PathBuf)>> = {
        let database = assets::database();
        settings
            .materials
            .iter()
            .map(|material| {
                MaterialMap::ALL
                    .into_iter()
                    .filter_map(|map| {
                        let entry = database.get(material.map(map)?)?;
                        Some((map, entry.source_path()))
                    })
                    .collect()
            })
            .collect()
    };
    let path = source.to_string_lossy().replace('\\', "/");
    let mut model = ModelBuilder::from_file(&path)
        .map_err(|e| e.to_string())?
        .with_scale(settings.scale)
        .build();
    model.init();
    for (material, maps) in overrides.into_iter().enumerate() {
        for (map, file) in maps {
            model.set_material_map_file(material, map, &file);
        }
    }
    let variant = settings.variants.get(variant).map(|v| v.name.as_str());
    model.show_variant(variant)?;
    Ok(model)
}

/// A model held in the hand of the player model, which stands still or
/// plays a clip.
struct Held {
    player: Rigged,
    item: Model,
    /// The clip played (index into the asset's hold poses + 1; 0 none).
    pose_index: usize,
    pose: Option<Animation>,
    /// Ticks into the clip.
    time: f32,
    last: Instant,
    /// What the camera turns around.
    center: Point3<f32>,
    radius: f32,
}

impl Held {
    fn load(edit: &AssetEdit, settings: &ModelSettings) -> Result<Held, String> {
        let player = {
            let database = assets::database();
            let source = database
                .root()
                .join("models")
                .join(ferrite::player::PLAYER_MODEL_FILE);
            database
                .find_by_source(&source)
                .and_then(|id| database.get(id))
                .and_then(|entry| Some((entry.source_path(), entry.meta.as_model()?.clone())))
        };
        let (source, player_settings) =
            player.ok_or("there is no player model to hold it (models/Mannequin.fbx)")?;
        let player = Rigged::load(&source, &player_settings, 0)?
            .ok_or("the player model has no bones")?;
        let item = load_model(&edit.source_path(), settings, edit.variant)?;
        // Between the body and the right hand, close enough to see the grip.
        let hand = player
            .skeleton
            .joints
            .iter()
            .position(|joint| ferrite::core::model::ik::base_name(&joint.name) == "RightHand")
            .map(|joint| player.skeleton.world_positions(&player.root())[joint]);
        let center = match hand {
            Some(hand) => Point3::from_vec((player.center.to_vec() + hand.to_vec()) * 0.5),
            None => player.center,
        };
        Ok(Held {
            radius: player.radius * 0.6,
            center,
            player,
            item,
            pose_index: 0,
            pose: None,
            time: 0.0,
            last: Instant::now(),
        })
    }

    /// Play `index` of the hold poses (0: stand in the rest pose).
    fn set_pose(&mut self, index: usize, poses: &[HoldPose]) {
        if index == self.pose_index {
            return;
        }
        self.pose_index = index;
        self.time = 0.0;
        self.pose = index.checked_sub(1).and_then(|i| poses.get(i)).and_then(|pose| {
            Animation::load(&pose.label, &pose.source, pose.clip.as_deref())
                .map_err(|e| ferrite::log::warn!("{}: {e}", pose.label))
                .ok()
        });
    }

    /// Move the clip on and pose the player model, the other hand on the
    /// grip.
    fn animate(&mut self, grip: &Grip) {
        let dt = self.last.elapsed().as_secs_f32().min(0.1);
        self.last = Instant::now();
        let player = &mut self.player;
        player.skeleton.reset();
        player.model.apply_skeleton(&player.skeleton);
        if let Some(pose) = &self.pose {
            let duration = pose.duration.max(f32::EPSILON);
            self.time = (self.time + dt * pose.ticks_per_second) % duration;
            player.model.apply_pose(&pose.sample(self.time));
            player.model.reset_position();
            // The root's movement is the character's (root motion, in the
            // game): it stands where it is.
            if let (Some(mut posed), Some(rest)) =
                (player.model.skeleton(), player.skeleton.joints.first())
            {
                if let Some(root) = posed.joints.first_mut() {
                    root.local = rest.local;
                }
                player.model.apply_skeleton(&posed);
            }
        }
        grip::reach_with_other_hand(
            &mut player.model,
            &Matrix4::identity(),
            grip,
            Hand::Right,
            1.0,
        );
    }

    /// Where the held model is drawn.
    fn placed(&self, grip: &Grip) -> Option<Matrix4<f32>> {
        grip::held_transform(&self.player.model, &Matrix4::identity(), grip, Hand::Right)
    }
}

/// A grip being moved in the preview.
enum GripDrag {
    /// The held model follows the cursor, sliding through the hand.
    Item {
        /// Where it was drawn when the drag began.
        placed: Matrix4<f32>,
        start_point: [f32; 3],
        plane_normal: Vector3<f32>,
        start_hit: Point3<f32>,
    },
    /// The other hand's point follows the cursor over the model.
    OtherHand {
        placed: Matrix4<f32>,
        plane_normal: Vector3<f32>,
        plane_point: Point3<f32>,
        grab: Vector3<f32>,
    },
}

impl Rigged {
    /// Load the model of the working copy skinned; `None` if it has no
    /// bones.
    fn load(
        source: &std::path::Path,
        settings: &ModelSettings,
        variant: usize,
    ) -> Result<Option<Rigged>, String> {
        // Texture files of the working copy's materials, found before the
        // (slow) load.
        let overrides: Vec<Vec<(MaterialMap, PathBuf)>> = {
            let database = assets::database();
            settings
                .materials
                .iter()
                .map(|material| {
                    MaterialMap::ALL
                        .into_iter()
                        .filter_map(|map| {
                            let entry = database.get(material.map(map)?)?;
                            Some((map, entry.source_path()))
                        })
                        .collect()
                })
                .collect()
        };
        let path = source.to_string_lossy().replace('\\', "/");
        let mut model = ModelBuilder::from_file(&path)
            .map_err(|e| e.to_string())?
            .with_scale(settings.scale)
            .build();
        model.init();
        let Some(skeleton) = model.skeleton() else {
            return Ok(None);
        };
        for (material, maps) in overrides.into_iter().enumerate() {
            for (map, file) in maps {
                model.set_material_map_file(material, map, &file);
            }
        }
        let variant = settings.variants.get(variant).map(|v| v.name.as_str());
        model.show_variant(variant)?;

        let vertices = model.get_vertices_scaled();
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        for v in &vertices {
            for axis in 0..3 {
                min[axis] = min[axis].min(v[axis]);
                max[axis] = max[axis].max(v[axis]);
            }
        }
        if vertices.is_empty() {
            (min, max) = ([0.0; 3], [1.0; 3]);
        }
        let center = Point3::new(
            (min[0] + max[0]) * 0.5,
            (min[1] + max[1]) * 0.5,
            (min[2] + max[2]) * 0.5,
        );
        let radius = (0..3)
            .map(|axis| (max[axis] - min[axis]) * 0.5)
            .fold(0.0f32, |sum, half| sum + half * half)
            .sqrt()
            .max(0.01);
        let chains = skeleton.default_chains();
        Ok(Some(Rigged {
            model,
            skeleton,
            chains,
            center,
            radius,
            floor: min[1],
        }))
    }

    fn root(&self) -> Matrix4<f32> {
        self.model.root_transform(&Matrix4::identity())
    }

    /// Whether a joint can be selected and keyed (importer helper nodes
    /// cannot).
    fn is_bone(&self, joint: usize) -> bool {
        !self.skeleton.joints[joint].name.contains("$AssimpFbx$")
    }

    /// Pose the skeleton as a clip has it at `time`.
    fn pose_from(&mut self, clip: &ferrite::core::model::clip_file::ClipFile, time: f32) {
        self.skeleton.reset();
        let pose = clip.sample(time);
        for joint in &mut self.skeleton.joints {
            if let Some(transform) = pose.get(&joint.name) {
                joint.local = transform.to_matrix();
            }
        }
        self.model.apply_skeleton(&self.skeleton);
    }

    /// The bones whose local transform differs from `start`, with it.
    fn changed_since(&self, start: &[Matrix4<f32>]) -> Vec<usize> {
        let close = |a: &Matrix4<f32>, b: &Matrix4<f32>| {
            (0..4).all(|c| (0..4).all(|r| (a[c][r] - b[c][r]).abs() < 1e-6))
        };
        self.skeleton
            .joints
            .iter()
            .enumerate()
            .filter(|(i, joint)| self.is_bone(*i) && start.get(*i).is_none_or(|s| !close(s, &joint.local)))
            .map(|(i, _)| i)
            .collect()
    }

    fn transform_of(&self, joint: usize) -> (String, BoneTransform) {
        let joint = &self.skeleton.joints[joint];
        (joint.name.clone(), BoneTransform::from_matrix(&joint.local))
    }

    /// World position of each chain's handle.
    fn handles(&self) -> Vec<Point3<f32>> {
        let positions = self.skeleton.world_positions(&self.root());
        self.chains.iter().map(|chain| positions[chain.end]).collect()
    }
}

/// A handle being dragged: its end effector follows the cursor on a plane
/// facing the camera.
struct IkDrag {
    chain: usize,
    /// Pose when the drag began; every solve starts from it.
    start: Vec<Matrix4<f32>>,
    plane_point: Point3<f32>,
    plane_normal: Vector3<f32>,
    /// From where the cursor hit the plane to the handle.
    grab: Vector3<f32>,
    /// Whether the handle was moved (a click without moving selects it).
    moved: bool,
}

/// A ring of the selected bone being dragged: the bone turns about the
/// ring's axis as the cursor goes round the bone.
struct RingDrag {
    joint: usize,
    axis: usize,
    start: Vec<Matrix4<f32>>,
    /// Screen angle of the cursor around the bone last frame.
    angle: f32,
    /// +1 when the axis points towards the camera.
    sign: f32,
}

/// Pixels from a joint within which a click selects it.
const JOINT_PICK_RADIUS: f32 = 8.0;
/// Pixels from a ring within which a press takes it.
const RING_PICK_RADIUS: f32 = 6.0;
const SELECTED_COLOR: (f32, f32, f32) = (0.55, 0.95, 0.55);

/// Pixels from a handle within which a click takes it.
const HANDLE_PICK_RADIUS: f32 = 12.0;
const BONE_COLOR: (f32, f32, f32) = (0.45, 0.75, 1.0);
const HANDLE_COLOR: (f32, f32, f32) = (1.0, 0.8, 0.2);
const HANDLE_HOT_COLOR: (f32, f32, f32) = (1.0, 0.4, 0.1);

/// The asset workspace's main area: a 3D view of the model with an orbit
/// camera, or the image of a texture.
pub struct AssetPreview {
    handle: UIElementHandle,
    state: Shared,
    panel: Panel,
    shown: Shown,
    /// `(asset, preview counter)` that is shown.
    shown_for: Option<(AssetId, u64)>,
    /// `(asset, variant)` the camera was framed for.
    framed_for: Option<(AssetId, usize)>,
    /// When a reload became necessary.
    pending_since: Option<Instant>,
    message: Text,
    hint: Text,
    orbit: Orbit,
    /// Whether the bones and IK handles of a rigged model are drawn.
    show_skeleton: DataSource<bool>,
    reset_pose: DataSource<bool>,
    skeleton_button: Button,
    reset_button: Button,
    /// Chain whose handle is under the cursor.
    hovered: Option<usize>,
    ik_drag: Option<IkDrag>,
    /// Joint selected for the rings (outside a clip; a clip keeps its own
    /// selection by name).
    selected: Option<usize>,
    hovered_joint: Option<usize>,
    hovered_ring: Option<usize>,
    ring_drag: Option<RingDrag>,
    /// A held model's grip being moved, and which grip handle is under the
    /// cursor (0: the model, 1: the other hand).
    grip_drag: Option<GripDrag>,
    hovered_grip: Option<u8>,
    /// A clip is open: the model is posed from it and the timeline shows.
    clip_mode: bool,
    timeline: Timeline,
    /// Bones posed since the last key (without auto key).
    touched: Vec<usize>,
    /// `(time, keys counter)` the model was last posed from the clip for.
    posed_for: Option<(f32, u64)>,
    last_tick: Instant,
    z: f32,
}

impl AssetPreview {
    pub fn new(state: Shared) -> Self {
        let mut panel = Panel::new("Preview".into(), Position::default(), Size::new(600.0, 400.0));
        panel.set_movable(false);
        panel.set_fixed_size(true);
        panel.set_flat(true);
        let show_skeleton = DataSource::new(true);
        let reset_pose = DataSource::new(false);
        let skeleton_flag = show_skeleton.clone();
        let mut skeleton_button = header_button(
            "Skeleton",
            icons::EYE,
            ButtonStyle::Secondary,
            Box::new(move |_| skeleton_flag.write(!skeleton_flag.read())),
        );
        skeleton_button.set_tooltip("Show the bones and the handles to pose them with");
        let reset_flag = reset_pose.clone();
        let mut reset_button = header_button(
            "Reset pose",
            icons::ROTATE_CW,
            ButtonStyle::Secondary,
            Box::new(move |_| reset_flag.write(true)),
        );
        reset_button.set_tooltip("Put every bone back where the file has it");
        let timeline = Timeline::new(state.clone());
        Self {
            handle: UIElementHandle::new(),
            state,
            panel,
            show_skeleton,
            reset_pose,
            skeleton_button,
            reset_button,
            hovered: None,
            ik_drag: None,
            selected: None,
            hovered_joint: None,
            hovered_ring: None,
            ring_drag: None,
            grip_drag: None,
            hovered_grip: None,
            clip_mode: false,
            timeline,
            touched: Vec::new(),
            posed_for: None,
            last_tick: Instant::now(),
            z: 0.0,
            shown: Shown::Nothing,
            shown_for: None,
            framed_for: None,
            pending_since: None,
            message: theme::text("", theme::FONT_SIZE, theme::TEXT_MUTED),
            hint: theme::text(
                "Drag to orbit, scroll to zoom",
                theme::FONT_SIZE_SMALL,
                theme::TEXT_DIM,
            ),
            orbit: Orbit {
                yaw: 0.6,
                pitch: 0.25,
                zoom: DEFAULT_ZOOM,
                dragging: None,
            },
        }
    }

    fn rect(&self) -> Rect {
        self.state.borrow().layout.viewport
    }

    /// The area below the panel header (above the timeline of a clip).
    fn view_rect(&self) -> Rect {
        let r = self.rect();
        let header = self.panel.header_height();
        let timeline = if self.clip_mode { TIMELINE_HEIGHT } else { 0.0 };
        Rect {
            x: r.x + 1.0,
            y: r.y + header,
            w: (r.w - 2.0).max(1.0),
            h: (r.h - header - 1.0 - timeline).max(1.0),
        }
    }

    fn timeline_rect(&self) -> Rect {
        let r = self.rect();
        let view = self.view_rect();
        Rect {
            x: view.x,
            y: view.y + view.h,
            w: view.w,
            h: (r.y + r.h - 1.0 - (view.y + view.h)).max(1.0),
        }
    }

    fn load(&mut self) {
        self.selected = None;
        self.touched.clear();
        self.posed_for = None;
        self.ring_drag = None;
        if self.load_clip() {
            return;
        }
        let state = self.state.borrow();
        let Some(edit) = &state.asset_edit else {
            self.shown = Shown::Nothing;
            return;
        };
        self.grip_drag = None;
        self.hovered_grip = None;
        let held = match &edit.meta.kind {
            AssetKind::Model(model) if edit.holding && model.grip.is_some() => {
                Some(Held::load(edit, model))
            }
            _ => None,
        };
        self.shown = match held {
            Some(Ok(held)) => Shown::Held(Box::new(held)),
            Some(Err(e)) => Shown::Failed(e),
            None => match &edit.meta.kind {
            AssetKind::Texture(_) => {
                let image = Image::new(
                    Position::default(),
                    Size::new(TEXTURE_SIDE, TEXTURE_SIDE),
                    edit.source_path(),
                );
                match image.error() {
                    Some(error) => Shown::Failed(error.to_string()),
                    None => Shown::Texture(image),
                }
            }
            AssetKind::Animation(animation) => Shown::Note(format!(
                "{} animation clip{}. Use them in an animation graph to see them play.",
                animation.clips.len(),
                if animation.clips.len() == 1 { "" } else { "s" }
            )),
            // Files that bring animation clips are drawn skinned when they
            // turn out to have bones.
            AssetKind::Model(model) if !model.clips.is_empty() => match Rigged::load(&edit.source_path(), model, edit.variant) {
                Ok(Some(rigged)) => Shown::Rigged(Box::new(rigged)),
                Ok(None) => Self::load_static(edit, model),
                Err(e) => Shown::Failed(e),
            },
            AssetKind::Model(model) => Self::load_static(edit, model),
            },
        };
        self.ik_drag = None;
        self.hovered = None;
        self.hint.set_content(match self.shown {
            Shown::Rigged(_) => {
                "Drag a handle to pose, elsewhere to orbit, scroll to zoom"
            }
            Shown::Held(_) => {
                "Drag the model to move it in the hand, the blue diamond for the other hand; elsewhere to orbit"
            }
            _ => "Drag to orbit, scroll to zoom",
        });
    }

    /// Show the model a clip is posed on; false if the asset is no clip.
    fn load_clip(&mut self) -> bool {
        let model = {
            let state = self.state.borrow();
            let Some(clip) = state.asset_edit.as_ref().and_then(|edit| edit.clip.as_ref()) else {
                return false;
            };
            let database = assets::database();
            let id = clip.file.model.or_else(|| {
                let source = database
                    .root()
                    .join("models")
                    .join(ferrite::player::PLAYER_MODEL_FILE);
                database.find_by_source(&source)
            });
            id.and_then(|id| database.get(id)).and_then(|entry| {
                Some((entry.source_path(), entry.meta.as_model()?.clone()))
            })
        };
        self.ik_drag = None;
        self.hovered = None;
        self.shown = match model {
            None => Shown::Failed("the model this clip is posed on does not exist".into()),
            Some((source, settings)) => match Rigged::load(&source, &settings, 0) {
                Ok(Some(rigged)) => Shown::Rigged(Box::new(rigged)),
                Ok(None) => Shown::Failed("the clip's model has no bones".into()),
                Err(e) => Shown::Failed(e),
            },
        };
        if let Shown::Rigged(rigged) = &self.shown {
            // Every node but the root (whose translation drives root motion
            // in game): clips from model files turn the importer's helper
            // nodes, so a clip that leaves them out would keep whatever the
            // clip before it left there.
            let rest: Vec<BonePose> = rigged
                .skeleton
                .joints
                .iter()
                .filter(|joint| joint.parent.is_some())
                .map(|joint| BonePose {
                    bone: joint.name.clone(),
                    pose: BoneTransform::from_matrix(&joint.local),
                })
                .collect();
            let bones = rigged
                .skeleton
                .joints
                .iter()
                .enumerate()
                .filter(|(i, _)| rigged.is_bone(*i))
                .map(|(_, joint)| joint.name.clone())
                .collect();
            let mut state = self.state.borrow_mut();
            if let Some(clip) = state.asset_edit.as_mut().and_then(|edit| edit.clip.as_mut()) {
                clip.rest = rest;
                clip.bones = bones;
                // The inspector lists the bones now.
                clip.selection += 1;
            }
        }
        self.hint.set_content(
            "Drag a handle or ring to pose, click a joint to select it, elsewhere to orbit",
        );
        true
    }

    /// The selected joint: the clip's selection by name, or the preview's.
    fn selected_joint(&self) -> Option<usize> {
        let Shown::Rigged(rigged) = &self.shown else { return None };
        if self.clip_mode {
            let state = self.state.borrow();
            let name = state.asset_edit.as_ref()?.clip.as_ref()?.selected.clone()?;
            rigged.skeleton.find(&name)
        } else {
            self.selected
        }
    }

    fn select_joint(&mut self, joint: Option<usize>) {
        let Shown::Rigged(rigged) = &self.shown else { return };
        if self.clip_mode {
            let name = joint.map(|j| rigged.skeleton.joints[j].name.clone());
            self.with_clip(|clip| clip.select(name));
        } else {
            self.selected = joint;
        }
    }

    /// Run `f` on the open clip.
    fn with_clip<R>(&self, f: impl FnOnce(&mut ClipEdit) -> R) -> Option<R> {
        let mut state = self.state.borrow_mut();
        let edit = state.asset_edit.as_mut()?;
        let clip = edit.clip.as_mut()?;
        Some(f(clip))
    }

    /// The joint nearest to a window position, within the pick radius.
    fn joint_at(&self, rigged: &Rigged, view: &Rect, x: f32, y: f32) -> Option<usize> {
        let vp = self.view_projection(view, rigged.center, rigged.radius);
        rigged
            .skeleton
            .world_positions(&rigged.root())
            .into_iter()
            .enumerate()
            .filter(|(i, _)| rigged.is_bone(*i))
            .filter_map(|(i, p)| {
                let (sx, sy) = project(&vp, view, p)?;
                let d = ((sx - x).powi(2) + (sy - y).powi(2)).sqrt();
                (d <= JOINT_PICK_RADIUS).then_some((i, d))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    fn ring_radius(rigged: &Rigged) -> f32 {
        rigged.radius * 0.13
    }

    /// The ring of the selected joint under a window position.
    fn ring_at(&self, rigged: &Rigged, view: &Rect, x: f32, y: f32) -> Option<usize> {
        let joint = self.selected_joint()?;
        let vp = self.view_projection(view, rigged.center, rigged.radius);
        let center = rigged.skeleton.world_positions(&rigged.root())[joint];
        let radius = Self::ring_radius(rigged);
        AXES.iter()
            .enumerate()
            .filter_map(|(axis, dir)| {
                let points: Vec<(f32, f32)> = ring_points(center, *dir, radius)
                    .into_iter()
                    .filter_map(|p| project(&vp, view, p))
                    .collect();
                let d = points
                    .windows(2)
                    .map(|w| distance_to_segment((x, y), w[0], w[1]))
                    .fold(f32::MAX, f32::min);
                (d <= RING_PICK_RADIUS).then_some((axis, d))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(axis, _)| axis)
    }

    /// Screen position of a joint.
    fn joint_on_screen(&self, rigged: &Rigged, view: &Rect, joint: usize) -> Option<(f32, f32)> {
        let vp = self.view_projection(view, rigged.center, rigged.radius);
        project(&vp, view, rigged.skeleton.world_positions(&rigged.root())[joint])
    }

    fn begin_ring_drag(&mut self, x: f32, y: f32) -> bool {
        let view = self.view_rect();
        let Shown::Rigged(rigged) = &self.shown else { return false };
        if !self.show_skeleton.read() {
            return false;
        }
        let Some(axis) = self.ring_at(rigged, &view, x, y) else { return false };
        let Some(joint) = self.selected_joint() else { return false };
        let Some((cx, cy)) = self.joint_on_screen(rigged, &view, joint) else { return false };
        let center = rigged.skeleton.world_positions(&rigged.root())[joint];
        let eye = self.eye(rigged.center, rigged.radius);
        let sign = if AXES[axis].dot(eye - center) >= 0.0 { 1.0 } else { -1.0 };
        self.ring_drag = Some(RingDrag {
            joint,
            axis,
            start: rigged.skeleton.locals(),
            angle: (-(y - cy)).atan2(x - cx),
            sign,
        });
        true
    }

    fn continue_ring_drag(&mut self, x: f32, y: f32) {
        let view = self.view_rect();
        let Some(drag) = &self.ring_drag else { return };
        let Shown::Rigged(rigged) = &self.shown else { return };
        let Some((cx, cy)) = self.joint_on_screen(rigged, &view, drag.joint) else { return };
        let angle = (-(y - cy)).atan2(x - cx);
        let mut delta = angle - drag.angle;
        while delta > std::f32::consts::PI {
            delta -= std::f32::consts::TAU;
        }
        while delta < -std::f32::consts::PI {
            delta += std::f32::consts::TAU;
        }
        let (joint, axis, sign) = (drag.joint, drag.axis, drag.sign);
        if let Some(drag) = &mut self.ring_drag {
            drag.angle = angle;
        }
        let Shown::Rigged(rigged) = &mut self.shown else { return };
        let turn = <cgmath::Quaternion<f32> as cgmath::Rotation3>::from_axis_angle(AXES[axis], cgmath::Rad(delta * sign));
        rigged.skeleton.turn(joint, turn);
        rigged.model.apply_skeleton(&rigged.skeleton);
    }

    /// A pose change ended: key the moved bones (auto key) or remember
    /// them for the next key.
    fn finish_pose(&mut self, start: &[Matrix4<f32>]) {
        let Shown::Rigged(rigged) = &self.shown else { return };
        let changed = rigged.changed_since(start);
        if changed.is_empty() || !self.clip_mode {
            return;
        }
        let poses: Vec<_> = changed.iter().map(|&j| rigged.transform_of(j)).collect();
        let auto_key = self.with_clip(|clip| clip.auto_key).unwrap_or(false);
        if auto_key {
            self.key_poses(&poses);
        } else {
            for joint in changed {
                if !self.touched.contains(&joint) {
                    self.touched.push(joint);
                }
            }
        }
    }

    fn key_poses(&mut self, poses: &[(String, BoneTransform)]) {
        if poses.is_empty() {
            return;
        }
        let mut state = self.state.borrow_mut();
        if let Some(edit) = state.asset_edit.as_mut() {
            if let Some(clip) = edit.clip.as_mut() {
                clip.key(poses);
                edit.dirty = true;
            }
        }
    }

    /// Carry out the requests of the timeline's buttons and shortcuts, move
    /// the playhead and pose the model for it.
    fn update_clip(&mut self) {
        let dt = self.last_tick.elapsed().as_secs_f32().min(0.1);
        self.last_tick = Instant::now();
        let requests = self
            .with_clip(|clip| std::mem::take(&mut clip.requests))
            .unwrap_or_default();
        for request in requests {
            match request {
                ClipRequest::TogglePlay => {
                    self.with_clip(|clip| clip.toggle_play());
                }
                ClipRequest::ToggleLoop => {
                    self.with_clip(|clip| clip.looping = !clip.looping);
                }
                ClipRequest::ToggleAutoKey => {
                    self.with_clip(|clip| clip.auto_key = !clip.auto_key);
                }
                ClipRequest::ToStart => {
                    self.with_clip(|clip| {
                        clip.playing = false;
                        clip.set_time(0.0);
                    });
                }
                ClipRequest::Key => {
                    let Shown::Rigged(rigged) = &self.shown else { continue };
                    let mut joints = self.touched.clone();
                    if let Some(selected) = self.selected_joint() {
                        if !joints.contains(&selected) {
                            joints.push(selected);
                        }
                    }
                    let poses: Vec<_> = joints.iter().map(|&j| rigged.transform_of(j)).collect();
                    self.touched.clear();
                    if poses.is_empty() {
                        self.state
                            .borrow_mut()
                            .set_status("Select a bone or pose one to key it");
                    }
                    self.key_poses(&poses);
                }
                ClipRequest::DeleteKey => {
                    let mut state = self.state.borrow_mut();
                    if let Some(edit) = state.asset_edit.as_mut() {
                        if edit.clip.as_mut().is_some_and(|clip| clip.delete_key()) {
                            edit.dirty = true;
                        }
                    }
                }
            }
        }
        let wanted = self.with_clip(|clip| {
            clip.advance(dt);
            (clip.time, clip.keys)
        });
        let dragging = self.ik_drag.is_some() || self.ring_drag.is_some();
        if let Some(wanted) = wanted {
            if !dragging && Some(wanted) != self.posed_for {
                let state = self.state.borrow();
                let clip = state.asset_edit.as_ref().and_then(|e| e.clip.as_ref());
                if let (Some(clip), Shown::Rigged(rigged)) = (clip, &mut self.shown) {
                    rigged.pose_from(&clip.file, clip.time);
                    self.posed_for = Some(wanted);
                    self.touched.clear();
                }
            }
        }
    }

    /// Act on what the user did in the timeline.
    fn timeline_action(&mut self, action: TimelineAction) {
        let mut state = self.state.borrow_mut();
        let Some(edit) = state.asset_edit.as_mut() else { return };
        let Some(clip) = edit.clip.as_mut() else { return };
        match action {
            TimelineAction::Scrub(time) => {
                clip.playing = false;
                clip.set_time(time);
            }
            TimelineAction::Select(bone) => clip.select(bone),
            TimelineAction::TakeKey { bone, time } => {
                clip.playing = false;
                clip.set_time(time);
                if bone.is_some() {
                    clip.select(bone);
                }
            }
            TimelineAction::MoveKey { bone, from, to } => {
                if (from - to).abs() > 1e-4 {
                    clip.move_key(bone.as_deref(), from, to);
                    clip.set_time(to);
                    edit.dirty = true;
                }
            }
        }
    }

    /// A model drawn the way the scatter system draws it.
    fn load_static(edit: &AssetEdit, model: &ModelSettings) -> Shown {
        {
            {
                let variant = model.variants.get(edit.variant).map(|v| v.name.as_str());
                match ScatterModel::load_preview(&edit.source_path(), model, variant) {
                    Ok(loaded) => {
                        let (min, max) = loaded.bounds(0).unwrap_or(([0.0; 3], [1.0; 3]));
                        let center = Point3::new(
                            (min[0] + max[0]) * 0.5,
                            (min[1] + max[1]) * 0.5,
                            (min[2] + max[2]) * 0.5,
                        );
                        let radius = (0..3)
                            .map(|axis| (max[axis] - min[axis]) * 0.5)
                            .fold(0.0f32, |sum, half| sum + half * half)
                            .sqrt()
                            .max(0.01);
                        let spec = ScatterSpec {
                            model: loaded,
                            attempts: 1,
                            salt: 0,
                        };
                        Shown::Model {
                            instances: ScatterInstances::new(&spec, &[(0.0, 0.0, 0.0, 0.0)]),
                            center,
                            radius,
                            floor: min[1],
                        }
                    }
                    Err(e) => Shown::Failed(e),
                }
            }
        }
    }

    /// Ray from the eye through a window position inside `view`.
    fn cursor_ray(
        &self,
        view: &Rect,
        center: Point3<f32>,
        radius: f32,
        x: f32,
        y: f32,
    ) -> Option<(Point3<f32>, Vector3<f32>)> {
        let inverse = self.view_projection(view, center, radius).invert()?;
        let ndc_x = (x - view.x) / view.w * 2.0 - 1.0;
        let ndc_y = 1.0 - (y - view.y) / view.h * 2.0;
        let unproject = |z: f32| {
            let p = inverse * Vector4::new(ndc_x, ndc_y, z, 1.0);
            Point3::from_vec(p.truncate() / p.w)
        };
        let near = unproject(-1.0);
        let far = unproject(1.0);
        Some((near, (far - near).normalize()))
    }

    /// The chain whose handle is nearest to a window position, within the
    /// pick radius.
    fn handle_at(&self, rigged: &Rigged, view: &Rect, x: f32, y: f32) -> Option<usize> {
        let vp = self.view_projection(view, rigged.center, rigged.radius);
        rigged
            .handles()
            .into_iter()
            .enumerate()
            .filter_map(|(i, p)| {
                let (sx, sy) = project(&vp, view, p)?;
                let d = ((sx - x).powi(2) + (sy - y).powi(2)).sqrt();
                (d <= HANDLE_PICK_RADIUS).then_some((i, d))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    /// Start dragging a handle if one is at the cursor.
    fn begin_ik_drag(&mut self, x: f32, y: f32) -> bool {
        let view = self.view_rect();
        let Shown::Rigged(rigged) = &self.shown else {
            return false;
        };
        if !self.show_skeleton.read() {
            return false;
        }
        let Some(chain) = self.handle_at(rigged, &view, x, y) else {
            return false;
        };
        let Some((origin, dir)) = self.cursor_ray(&view, rigged.center, rigged.radius, x, y) else {
            return false;
        };
        let handle = rigged.handles()[chain];
        let normal = (rigged.center - self.eye(rigged.center, rigged.radius)).normalize();
        let Some(hit) = ray_plane(origin, dir, handle, normal) else {
            return false;
        };
        self.ik_drag = Some(IkDrag {
            chain,
            start: rigged.skeleton.locals(),
            plane_point: handle,
            plane_normal: normal,
            grab: handle - hit,
            moved: false,
        });
        true
    }

    /// Move the dragged handle to the cursor and pose the model for it.
    fn continue_ik_drag(&mut self, x: f32, y: f32) {
        let view = self.view_rect();
        let Some(drag) = &self.ik_drag else { return };
        let Shown::Rigged(rigged) = &self.shown else { return };
        let Some((origin, dir)) = self.cursor_ray(&view, rigged.center, rigged.radius, x, y) else {
            return;
        };
        let Some(hit) = ray_plane(origin, dir, drag.plane_point, drag.plane_normal) else {
            return;
        };
        let Some(to_skeleton) = rigged.root().inverse_transform() else { return };
        let target = to_skeleton.transform_point(hit + drag.grab);
        let (chain, start) = (drag.chain, drag.start.clone());
        if let Some(drag) = &mut self.ik_drag {
            drag.moved = true;
        }
        let Shown::Rigged(rigged) = &mut self.shown else { return };
        rigged.skeleton.set_locals(&start);
        let ik_chain = rigged.chains[chain].clone();
        rigged.skeleton.solve(&ik_chain, target);
        rigged.model.apply_skeleton(&rigged.skeleton);
    }

    /// The grip of the working copy.
    fn grip(&self) -> Option<Grip> {
        self.state.borrow().asset_edit.as_ref()?.grip().cloned()
    }

    fn set_grip(&self, grip: Grip) {
        if let Some(edit) = self.state.borrow_mut().asset_edit.as_mut() {
            edit.set_grip(Some(grip));
        }
    }

    /// Where the grip's handles are: the holding hand's point and the
    /// other hand's, in world space.
    fn grip_handles(held: &Held, grip: &Grip) -> Option<(Point3<f32>, Option<Point3<f32>>)> {
        let placed = held.placed(grip)?;
        let point = placed.transform_point(grip.point.into());
        let other = grip.other_hand.map(|p| placed.transform_point(p.into()));
        Some((point, other))
    }

    /// The grip handle at a window position: 1 the other hand's diamond,
    /// 0 the held model (its diamond, or the model itself).
    fn grip_handle_at(&self, held: &Held, view: &Rect, x: f32, y: f32) -> Option<u8> {
        let grip = self.grip()?;
        let (point, other) = Self::grip_handles(held, &grip)?;
        let vp = self.view_projection(view, held.center, held.radius);
        let near = |p: Point3<f32>| {
            project(&vp, view, p)
                .is_some_and(|(sx, sy)| ((sx - x).powi(2) + (sy - y).powi(2)).sqrt() <= HANDLE_PICK_RADIUS)
        };
        if other.is_some_and(near) {
            return Some(1);
        }
        if near(point) {
            return Some(0);
        }
        let placed = held.placed(&grip)?;
        let (origin, dir) = self.cursor_ray(view, held.center, held.radius, x, y)?;
        held.item.raycast(&placed, origin, dir).map(|_| 0)
    }

    /// Start moving the grip if the cursor is on one of its handles.
    fn begin_grip_drag(&mut self, x: f32, y: f32) -> bool {
        let view = self.view_rect();
        let Shown::Held(held) = &self.shown else {
            return false;
        };
        let Some(grip) = self.grip() else { return false };
        let Some(which) = self.grip_handle_at(held, &view, x, y) else {
            return false;
        };
        let (Some(placed), Some((point, other))) = (held.placed(&grip), Self::grip_handles(held, &grip))
        else {
            return false;
        };
        let Some((origin, dir)) = self.cursor_ray(&view, held.center, held.radius, x, y) else {
            return false;
        };
        let normal = (held.center - self.eye(held.center, held.radius)).normalize();
        self.grip_drag = match (which, other) {
            (1, Some(other)) => {
                let Some(hit) = ray_plane(origin, dir, other, normal) else { return false };
                Some(GripDrag::OtherHand {
                    placed,
                    plane_normal: normal,
                    plane_point: other,
                    grab: other - hit,
                })
            }
            _ => {
                let Some(hit) = ray_plane(origin, dir, point, normal) else { return false };
                Some(GripDrag::Item {
                    placed,
                    start_point: grip.point,
                    plane_normal: normal,
                    start_hit: hit,
                })
            }
        };
        true
    }

    /// Move the grip as the cursor goes.
    fn continue_grip_drag(&mut self, x: f32, y: f32) {
        let view = self.view_rect();
        let (Some(drag), Shown::Held(held)) = (&self.grip_drag, &self.shown) else {
            return;
        };
        let Some(mut grip) = self.grip() else { return };
        let Some((origin, dir)) = self.cursor_ray(&view, held.center, held.radius, x, y) else {
            return;
        };
        match drag {
            GripDrag::Item {
                placed,
                start_point,
                plane_normal,
                start_hit,
            } => {
                let Some(hit) = ray_plane(origin, dir, *start_hit, *plane_normal) else { return };
                let Some(inverse) = placed.inverse_transform() else { return };
                // The model moves with the cursor: the hand holds it where
                // it was that much the other way.
                let moved = inverse.transform_vector(hit - *start_hit);
                let point = Vector3::from(*start_point) - moved;
                grip.point = [point.x, point.y, point.z].map(rounded);
            }
            GripDrag::OtherHand {
                placed,
                plane_normal,
                plane_point,
                grab,
            } => {
                let Some(hit) = ray_plane(origin, dir, *plane_point, *plane_normal) else { return };
                let Some(inverse) = placed.inverse_transform() else { return };
                let p = inverse.transform_point(hit + *grab);
                grip.other_hand = Some([p.x, p.y, p.z].map(rounded));
            }
        }
        self.set_grip(grip);
    }

    fn eye(&self, center: Point3<f32>, radius: f32) -> Point3<f32> {
        let o = &self.orbit;
        let distance = radius * o.zoom;
        center
            + Vector3::new(
                distance * o.pitch.cos() * o.yaw.sin(),
                distance * o.pitch.sin(),
                distance * o.pitch.cos() * o.yaw.cos(),
            )
    }

    fn view_projection(&self, view: &Rect, center: Point3<f32>, radius: f32) -> Matrix4<f32> {
        let eye = self.eye(center, radius);
        let projection = perspective(
            Deg(38.0),
            view.w / view.h.max(1.0),
            (radius * 0.02).max(0.001),
            radius * 40.0,
        );
        projection * Matrix4::look_at_rh(eye, center, Vector3::unit_y())
    }
}

impl UIElement for AssetPreview {
    fn update(&mut self, scene: &mut Scene) {
        let rect = self.rect();
        self.panel.set_size(Size::new(rect.w, rect.h));
        self.panel.set_position(Position::new(rect.x, rect.y, 0.0));

        let wanted = {
            let state = self.state.borrow();
            state
                .asset_edit
                .as_ref()
                .map(|edit| (edit.meta.id, edit.preview, edit.variant))
        };
        let wanted_key = wanted.map(|(id, preview, _)| (id, preview));
        if wanted_key != self.shown_for {
            // Another asset shows at once; edits of the same one wait until
            // they have settled.
            let same_asset = matches!(
                (wanted_key, self.shown_for),
                (Some((a, _)), Some((b, _))) if a == b
            );
            let since = *self.pending_since.get_or_insert_with(Instant::now);
            if !same_asset || since.elapsed().as_secs_f32() >= RELOAD_DELAY {
                self.load();
                self.shown_for = wanted_key;
                self.pending_since = None;
            }
        }
        // A fresh camera for another asset or variant, not for every edit.
        let frame_key = wanted.map(|(id, _, variant)| (id, variant));
        if frame_key != self.framed_for {
            self.framed_for = frame_key;
            self.orbit.yaw = 0.6;
            self.orbit.pitch = 0.25;
            self.orbit.zoom = DEFAULT_ZOOM;
        }

        self.clip_mode = matches!(self.shown, Shown::Rigged(_))
            && self
                .state
                .borrow()
                .asset_edit
                .as_ref()
                .is_some_and(|edit| edit.clip.is_some());
        if self.clip_mode {
            self.update_clip();
            let view = {
                let state = self.state.borrow();
                state
                    .asset_edit
                    .as_ref()
                    .and_then(|edit| edit.clip.as_ref())
                    .map(|clip| TimelineView::of(clip, &clip.bones))
                    .unwrap_or_default()
            };
            let rect = self.timeline_rect();
            self.timeline.update(scene, rect, view, self.z + 2.0);
        }
        if let Shown::Held(held) = &mut self.shown {
            let state = self.state.borrow();
            if let Some(edit) = &state.asset_edit {
                held.set_pose(edit.hold_pose, &edit.hold_poses);
                if let Some(grip) = edit.grip() {
                    held.animate(grip);
                }
            }
        }
        let view = self.view_rect();
        let message = match &self.shown {
            Shown::Nothing => "No asset is open.".to_string(),
            Shown::Failed(error) => format!("The asset could not be loaded: {error}"),
            Shown::Note(note) => note.clone(),
            Shown::Texture(image) => image
                .natural_size()
                .map(|(w, h)| format!("{w} x {h} px"))
                .unwrap_or_default(),
            Shown::Model { .. } | Shown::Held(_) => String::new(),
            Shown::Rigged(rigged) if rigged.chains.is_empty() => {
                "No feet, hands or head found among the bones to pose.".to_string()
            }
            Shown::Rigged(_) => String::new(),
        };
        if self.reset_pose.read() {
            self.reset_pose.write(false);
            self.ik_drag = None;
            self.ring_drag = None;
            if let Shown::Rigged(rigged) = &mut self.shown {
                let start = rigged.skeleton.locals();
                rigged.skeleton.reset();
                rigged.model.apply_skeleton(&rigged.skeleton);
                // In a clip, going back to the rest pose is posing too.
                self.finish_pose(&start);
            }
        }
        if matches!(self.shown, Shown::Rigged(_)) {
            let shown = self.show_skeleton.read();
            self.skeleton_button.set_active(shown);
            self.skeleton_button
                .set_icon(if shown { icons::EYE } else { icons::EYE_OFF });
            let mut x = view.x + 8.0;
            for button in [&mut self.skeleton_button, &mut self.reset_button] {
                button.set_offset(Offset { x, y: view.y + 8.0 });
                x += button.get_size().width + 4.0;
                button.update(scene);
            }
        }
        self.message.set_content(&message);
        self.message
            .prepare_render_at(Position::new(view.x + 16.0, view.y + 16.0, 3.0));
        self.hint.prepare_render_at(Position::new(
            view.x + 12.0,
            view.y + view.h - 24.0,
            3.0,
        ));
        if let Shown::Texture(image) = &mut self.shown {
            // As large as fits, keeping the image's proportions.
            let (w, h) = image.natural_size().unwrap_or((1, 1));
            let (w, h) = (w.max(1) as f32, h.max(1) as f32);
            let room = ((view.w - 48.0).max(16.0), (view.h - 72.0).max(16.0));
            let scale = (TEXTURE_SIDE / w.max(h)).min(room.0 / w).min(room.1 / h);
            let (w, h) = ((w * scale).round(), (h * scale).round());
            image.set_size(Size::new(w, h));
            image.set_offset(Offset {
                x: (view.x + (view.w - w) * 0.5).round(),
                y: (view.y + (view.h - h) * 0.5).round(),
            });
        }
        self.panel.update(scene);
    }

    fn render(&self) {
        self.panel.render();
        let view = self.view_rect();
        match &self.shown {
            Shown::Model {
                instances,
                center,
                radius,
                floor,
            } => {
                let (window_w, window_h, lod) = {
                    let state = self.state.borrow();
                    (
                        state.layout.window_w,
                        state.layout.window_h,
                        state.asset_edit.as_ref().and_then(|edit| edit.lod),
                    )
                };
                let gl_rect = [
                    view.x as i32,
                    (window_h - view.y - view.h) as i32,
                    view.w as i32,
                    view.h as i32,
                ];
                unsafe {
                    gl::Enable(gl::SCISSOR_TEST);
                    gl::Scissor(gl_rect[0], gl_rect[1], gl_rect[2], gl_rect[3]);
                    gl::Viewport(gl_rect[0], gl_rect[1], gl_rect[2], gl_rect[3]);
                    let bg = theme::BG_FIELD;
                    gl::ClearColor(bg.0, bg.1, bg.2, 1.0);
                    gl::Clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT);
                    gl::Enable(gl::DEPTH_TEST);
                    gl::DepthMask(gl::TRUE);
                    gl::DepthFunc(gl::LESS);
                }
                let vp = self.view_projection(&view, *center, *radius);

                // Floor grid under the model, sized to it.
                let extent = radius * 1.6;
                let steps = 12;
                let mut grid = Vec::new();
                for i in 0..=steps {
                    let t = -extent + 2.0 * extent * i as f32 / steps as f32;
                    grid.push(Line::new(
                        Point3::new(center.x + t, *floor, center.z - extent),
                        Vector3::unit_z(),
                        extent * 2.0,
                    ));
                    grid.push(Line::new(
                        Point3::new(center.x - extent, *floor, center.z + t),
                        Vector3::unit_x(),
                        extent * 2.0,
                    ));
                }
                LineRenderer::render_lines(&vp, &grid, Vector3::new(0.16, 0.17, 0.2), false);

                let lighting = Lighting::preview(
                    Vector3::new(3.0, 5.0, 4.0),
                    self.eye(*center, *radius),
                );
                let lod = match lod {
                    Some(tier) => LodChoice::Fixed(tier),
                    // Close up, as the camera never gets far in model terms.
                    None => LodChoice::Distance(0.0),
                };
                instances.draw(&vp, &Matrix4::identity(), &lighting, lod);
                unsafe {
                    // Leave no 3D depth behind for the UI drawn over this
                    // region.
                    gl::Clear(gl::DEPTH_BUFFER_BIT);
                    gl::Disable(gl::SCISSOR_TEST);
                    gl::Viewport(0, 0, window_w as i32, window_h as i32);
                }
                self.hint.render();
            }
            Shown::Rigged(rigged) => {
                let (window_w, window_h) = {
                    let state = self.state.borrow();
                    (state.layout.window_w, state.layout.window_h)
                };
                let (center, radius) = (rigged.center, rigged.radius);
                unsafe {
                    gl::Enable(gl::SCISSOR_TEST);
                    gl::Scissor(
                        view.x as i32,
                        (window_h - view.y - view.h) as i32,
                        view.w as i32,
                        view.h as i32,
                    );
                    gl::Viewport(
                        view.x as i32,
                        (window_h - view.y - view.h) as i32,
                        view.w as i32,
                        view.h as i32,
                    );
                    let bg = theme::BG_FIELD;
                    gl::ClearColor(bg.0, bg.1, bg.2, 1.0);
                    gl::Clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT);
                    gl::Enable(gl::DEPTH_TEST);
                    gl::DepthMask(gl::TRUE);
                    gl::DepthFunc(gl::LESS);
                }
                let vp = self.view_projection(&view, center, radius);
                let extent = radius * 1.6;
                let steps = 12;
                let mut grid = Vec::new();
                for i in 0..=steps {
                    let t = -extent + 2.0 * extent * i as f32 / steps as f32;
                    grid.push(Line::new(
                        Point3::new(center.x + t, rigged.floor, center.z - extent),
                        Vector3::unit_z(),
                        extent * 2.0,
                    ));
                    grid.push(Line::new(
                        Point3::new(center.x - extent, rigged.floor, center.z + t),
                        Vector3::unit_x(),
                        extent * 2.0,
                    ));
                }
                LineRenderer::render_lines(&vp, &grid, Vector3::new(0.16, 0.17, 0.2), false);
                unsafe { gl::Enable(gl::DEPTH_TEST) };
                let lighting =
                    Lighting::preview(Vector3::new(3.0, 5.0, 4.0), self.eye(center, radius));
                rigged.model.render(&lighting, &Matrix4::identity(), &vp);
                if self.show_skeleton.read() {
                    let hot = self.ik_drag.as_ref().map(|d| d.chain).or(self.hovered);
                    render_skeleton(rigged, &vp, &view, hot);
                    let positions = rigged.skeleton.world_positions(&rigged.root());
                    let size = rigged.radius * 0.012;
                    if let Some(joint) = self.hovered_joint {
                        render_thick_lines(&vp, &diamond(positions[joint], size * 1.6), HANDLE_HOT_COLOR, &view, 2);
                    }
                    if let Some(joint) = self.selected_joint() {
                        render_thick_lines(&vp, &diamond(positions[joint], size * 2.0), SELECTED_COLOR, &view, 2);
                        let radius = Self::ring_radius(rigged);
                        let hot = self.ring_drag.as_ref().map(|d| d.axis).or(self.hovered_ring);
                        for (axis, dir) in AXES.iter().enumerate() {
                            let points = ring_points(positions[joint], *dir, radius);
                            let lines: Vec<Line> =
                                points.windows(2).filter_map(|w| segment(w[0], w[1])).collect();
                            let (r, g, b) = RING_COLORS[axis];
                            let color = if Some(axis) == hot {
                                (1.0, 0.9, 0.4)
                            } else {
                                (r, g, b)
                            };
                            render_thick_lines(&vp, &lines, color, &view, 2);
                        }
                    }
                }
                unsafe {
                    gl::Clear(gl::DEPTH_BUFFER_BIT);
                    gl::Disable(gl::SCISSOR_TEST);
                    gl::Viewport(0, 0, window_w as i32, window_h as i32);
                }
                if self.clip_mode {
                    self.timeline.render((window_w, window_h));
                }
                self.hint.render();
                self.message.render();
                self.skeleton_button.render();
                self.reset_button.render();
            }
            Shown::Held(held) => {
                let (window_w, window_h) = {
                    let state = self.state.borrow();
                    (state.layout.window_w, state.layout.window_h)
                };
                let (center, radius) = (held.center, held.radius);
                unsafe {
                    gl::Enable(gl::SCISSOR_TEST);
                    gl::Scissor(
                        view.x as i32,
                        (window_h - view.y - view.h) as i32,
                        view.w as i32,
                        view.h as i32,
                    );
                    gl::Viewport(
                        view.x as i32,
                        (window_h - view.y - view.h) as i32,
                        view.w as i32,
                        view.h as i32,
                    );
                    let bg = theme::BG_FIELD;
                    gl::ClearColor(bg.0, bg.1, bg.2, 1.0);
                    gl::Clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT);
                    gl::Enable(gl::DEPTH_TEST);
                    gl::DepthMask(gl::TRUE);
                    gl::DepthFunc(gl::LESS);
                }
                let vp = self.view_projection(&view, center, radius);
                let floor = held.player.floor;
                let extent = held.player.radius * 1.2;
                let steps = 12;
                let mut grid = Vec::new();
                for i in 0..=steps {
                    let t = -extent + 2.0 * extent * i as f32 / steps as f32;
                    grid.push(Line::new(Point3::new(t, floor, -extent), Vector3::unit_z(), extent * 2.0));
                    grid.push(Line::new(Point3::new(-extent, floor, t), Vector3::unit_x(), extent * 2.0));
                }
                LineRenderer::render_lines(&vp, &grid, Vector3::new(0.16, 0.17, 0.2), false);
                unsafe { gl::Enable(gl::DEPTH_TEST) };
                let lighting = Lighting::preview(Vector3::new(3.0, 5.0, 4.0), self.eye(center, radius));
                held.player.model.render(&lighting, &Matrix4::identity(), &vp);
                if let Some(grip) = self.grip() {
                    if let Some(placed) = held.placed(&grip) {
                        held.item.render(&lighting, &placed, &vp);
                        let size = 0.012;
                        let hot = match &self.grip_drag {
                            Some(GripDrag::Item { .. }) => Some(0),
                            Some(GripDrag::OtherHand { .. }) => Some(1),
                            None => self.hovered_grip,
                        };
                        if let Some((point, other)) = Self::grip_handles(held, &grip) {
                            let color = if hot == Some(0) { HANDLE_HOT_COLOR } else { HANDLE_COLOR };
                            render_thick_lines(&vp, &diamond(point, size * 1.4), color, &view, 2);
                            // Towards the head and the edge.
                            let head = placed.transform_vector(Vector3::from(grip.along).normalize());
                            if let Some(line) = segment(point, point + head * 0.15) {
                                render_thick_lines(&vp, &vec![line], (1.0, 0.55, 0.2), &view, 2);
                            }
                            if let Some(other) = other {
                                let color = if hot == Some(1) { HANDLE_HOT_COLOR } else { BONE_COLOR };
                                render_thick_lines(&vp, &diamond(other, size * 1.4), color, &view, 2);
                            }
                        }
                    }
                }
                unsafe {
                    gl::Clear(gl::DEPTH_BUFFER_BIT);
                    gl::Disable(gl::SCISSOR_TEST);
                    gl::Viewport(0, 0, window_w as i32, window_h as i32);
                }
                self.hint.render();
                self.message.render();
            }
            Shown::Texture(image) => {
                image.render();
                self.message.render();
            }
            Shown::Nothing | Shown::Note(_) | Shown::Failed(_) => {
                self.message.render();
            }
        }
    }

    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut glfw::Window,
        glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        let view = self.view_rect();
        if self.clip_mode {
            let (used, action) = self.timeline.handle_events(scene, window, glfw, event);
            if let Some(action) = action {
                self.timeline_action(action);
            }
            if used {
                return true;
            }
        }
        if matches!(self.shown, Shown::Rigged(_)) {
            for button in [&mut self.skeleton_button, &mut self.reset_button] {
                if button.handle_events(scene, window, glfw, event)
                    && !matches!(event, WindowEvent::CursorPos(..))
                {
                    return true;
                }
            }
        }
        match event {
            WindowEvent::MouseButton(MouseButton::Button1, Action::Press, _) => {
                let (x, y) = window.get_cursor_pos();
                if view.contains(x as f32, y as f32) {
                    let (fx, fy) = (x as f32, y as f32);
                    if self.begin_grip_drag(fx, fy)
                        || self.begin_ring_drag(fx, fy)
                        || self.begin_ik_drag(fx, fy)
                    {
                        return true;
                    }
                    let joint = match &self.shown {
                        Shown::Rigged(rigged) if self.show_skeleton.read() => {
                            self.joint_at(rigged, &view, fx, fy)
                        }
                        _ => None,
                    };
                    match joint {
                        Some(joint) => self.select_joint(Some(joint)),
                        None => self.orbit.dragging = Some((x, y)),
                    }
                    return true;
                }
            }
            WindowEvent::MouseButton(MouseButton::Button1, Action::Release, _) => {
                if self.grip_drag.take().is_some() {
                    // The grip's fields show the new values.
                    if let Some(edit) = self.state.borrow_mut().asset_edit.as_mut() {
                        edit.structure += 1;
                    }
                    return true;
                }
                if let Some(drag) = self.ik_drag.take() {
                    if drag.moved {
                        self.finish_pose(&drag.start);
                    } else if let Shown::Rigged(rigged) = &self.shown {
                        // A click on a handle selects its bone.
                        let end = rigged.chains[drag.chain].end;
                        self.select_joint(Some(end));
                    }
                    return true;
                }
                if let Some(drag) = self.ring_drag.take() {
                    self.finish_pose(&drag.start);
                    return true;
                }
                if self.orbit.dragging.take().is_some() {
                    return true;
                }
            }
            WindowEvent::CursorPos(x, y) => {
                if self.grip_drag.is_some() {
                    self.continue_grip_drag(*x as f32, *y as f32);
                    return true;
                }
                self.hovered_grip = match &self.shown {
                    Shown::Held(held)
                        if self.orbit.dragging.is_none() && view.contains(*x as f32, *y as f32) =>
                    {
                        self.grip_handle_at(held, &view, *x as f32, *y as f32)
                    }
                    _ => None,
                };
                if self.ik_drag.is_some() {
                    self.continue_ik_drag(*x as f32, *y as f32);
                    return true;
                }
                if self.ring_drag.is_some() {
                    self.continue_ring_drag(*x as f32, *y as f32);
                    return true;
                }
                let free = self.orbit.dragging.is_none() && view.contains(*x as f32, *y as f32);
                (self.hovered_ring, self.hovered_joint) = match &self.shown {
                    Shown::Rigged(rigged) if self.show_skeleton.read() && free => {
                        let ring = self.ring_at(rigged, &view, *x as f32, *y as f32);
                        let joint = if ring.is_none() {
                            self.joint_at(rigged, &view, *x as f32, *y as f32)
                        } else {
                            None
                        };
                        (ring, joint)
                    }
                    _ => (None, None),
                };
                self.hovered = match &self.shown {
                    Shown::Rigged(rigged)
                        if self.show_skeleton.read()
                            && self.orbit.dragging.is_none()
                            && view.contains(*x as f32, *y as f32) =>
                    {
                        self.handle_at(rigged, &view, *x as f32, *y as f32)
                    }
                    _ => None,
                };
                if let Some((lx, ly)) = self.orbit.dragging {
                    self.orbit.yaw -= (*x - lx) as f32 * 0.01;
                    self.orbit.pitch =
                        (self.orbit.pitch + (*y - ly) as f32 * 0.01).clamp(-1.4, 1.4);
                    self.orbit.dragging = Some((*x, *y));
                    return true;
                }
            }
            WindowEvent::Scroll(_, dy) => {
                let (x, y) = window.get_cursor_pos();
                if view.contains(x as f32, y as f32) {
                    self.orbit.zoom = (self.orbit.zoom * (1.0 - *dy as f32 * 0.1)).clamp(1.2, 12.0);
                    return true;
                }
            }
            _ => {}
        }
        if let WindowEvent::MouseButton(..) | WindowEvent::Scroll(..) = event {
            let (x, y) = window.get_cursor_pos();
            return self.rect().contains(x as f32, y as f32);
        }
        false
    }

    fn add_child(&mut self, _child: Box<dyn UIElement>) {}
    fn add_child_to(&mut self, _p: UIElementHandle, _c: Box<dyn UIElement>) {}
    fn contains_child(&self, _h: &UIElementHandle) -> bool {
        false
    }
    fn get_child(&self, _h: &UIElementHandle) -> Option<&Box<dyn UIElement>> {
        None
    }
    fn get_child_mut(&mut self, _h: &UIElementHandle) -> Option<&mut Box<dyn UIElement>> {
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
    fn render_overlay(&self) {
        if self.clip_mode {
            self.timeline.render_overlay();
        }
    }

    fn set_z_index(&mut self, z: f32) {
        self.z = z;
        self.panel.set_z_index(z);
        self.message.set_z_index(z + 3.0);
        self.hint.set_z_index(z + 3.0);
        self.skeleton_button.set_z_index(z + 3.0);
        self.reset_button.set_z_index(z + 3.0);
    }
}

/// Window position of a world point inside `view`; `None` behind the eye.
fn project(vp: &Matrix4<f32>, view: &Rect, p: Point3<f32>) -> Option<(f32, f32)> {
    let clip = vp * p.to_homogeneous();
    if clip.w <= 1e-6 {
        return None;
    }
    let (x, y) = (clip.x / clip.w, clip.y / clip.w);
    Some((
        view.x + (x * 0.5 + 0.5) * view.w,
        view.y + (0.5 - y * 0.5) * view.h,
    ))
}

fn ray_plane(
    origin: Point3<f32>,
    dir: Vector3<f32>,
    point: Point3<f32>,
    normal: Vector3<f32>,
) -> Option<Point3<f32>> {
    let denom = dir.dot(normal);
    if denom.abs() < 1e-6 {
        return None;
    }
    let t = (point - origin).dot(normal) / denom;
    (t > 0.0).then(|| origin + dir * t)
}

/// Lines drawn on top of everything, `width` pixels wide.
fn render_thick_lines(vp: &Matrix4<f32>, lines: &Vec<Line>, color: (f32, f32, f32), view: &Rect, width: i32) {
    let color = Vector3::new(color.0, color.1, color.2);
    let half = width / 2;
    for dy in -half..=half {
        for dx in -half..=half {
            // A translation after the projection shifts NDC by exactly it.
            let shift = Matrix4::from_translation(Vector3::new(
                dx as f32 * 2.0 / view.w.max(1.0),
                dy as f32 * 2.0 / view.h.max(1.0),
                0.0,
            ));
            LineRenderer::render_lines(&(shift * vp), lines, color, true);
        }
    }
}

fn segment(from: Point3<f32>, to: Point3<f32>) -> Option<Line> {
    let d = to - from;
    let length = d.magnitude();
    (length > 1e-6).then(|| Line::new(from, d / length, length))
}

/// A wireframe diamond around `center`, `size` from its middle to a tip.
fn diamond(center: Point3<f32>, size: f32) -> Vec<Line> {
    let tips = [
        Vector3::unit_x(),
        -Vector3::unit_x(),
        Vector3::unit_z(),
        -Vector3::unit_z(),
    ];
    let mut lines = Vec::new();
    for (i, a) in tips.iter().enumerate() {
        let b = tips[[2, 3, 1, 0][i]];
        lines.extend(segment(center + a * size, center + b * size));
        lines.extend(segment(center + a * size, center + Vector3::unit_y() * size));
        lines.extend(segment(center + a * size, center - Vector3::unit_y() * size));
    }
    lines
}

/// Draw the bones of a rigged model and the handles of its chains.
fn render_skeleton(rigged: &Rigged, vp: &Matrix4<f32>, view: &Rect, hot: Option<usize>) {
    let positions = rigged.skeleton.world_positions(&rigged.root());
    let bones: Vec<Line> = rigged
        .skeleton
        .joints
        .iter()
        .enumerate()
        .filter_map(|(i, joint)| segment(positions[joint.parent?], positions[i]))
        .collect();
    render_thick_lines(vp, &bones, BONE_COLOR, view, 2);
    let size = rigged.radius * 0.035;
    for (i, chain) in rigged.chains.iter().enumerate() {
        let color = if Some(i) == hot { HANDLE_HOT_COLOR } else { HANDLE_COLOR };
        render_thick_lines(vp, &diamond(positions[chain.end], size), color, view, 2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrite::core::assets::{MaterialSettings, ModelSettings, ASSET_META_VERSION};

    fn edit() -> AssetEdit {
        AssetEdit {
            meta: AssetMeta {
                version: ASSET_META_VERSION,
                id: AssetId::from(5),
                name: "pack".into(),
                source: "pack.gltf".into(),
                kind: AssetKind::Model(ModelSettings {
                    scale: 1.0,
                    variants: Vec::new(),
                    materials: vec![MaterialSettings {
                        name: "bark".into(),
                        ..MaterialSettings::default()
                    }],
                    clips: Vec::new(),
                    grip: None,
                }),
            },
            dir: PathBuf::from("assets/models/test"),
            dirty: false,
            variant: 0,
            lod: None,
            structure: 0,
            preview: 0,
            clip: None,
            holding: false,
            hold_poses: Vec::new(),
            hold_pose: 0,
        }
    }

    #[test]
    fn giving_a_material_a_texture_marks_the_asset_changed() {
        let mut edit = edit();
        let texture = AssetId::from(9);
        edit.set_texture(0, TextureSlot::Normal, Some(texture));
        let material = &edit.meta.as_model().unwrap().materials[0];
        assert_eq!(material.normal, Some(texture));
        assert_eq!(material.base_color, None);
        assert!(edit.dirty);
        assert_eq!((edit.structure, edit.preview), (1, 1));

        // The same texture again is not a change.
        edit.dirty = false;
        edit.set_texture(0, TextureSlot::Normal, Some(texture));
        assert!(!edit.dirty);
        assert_eq!((edit.structure, edit.preview), (1, 1));

        // A material that does not exist is ignored.
        edit.set_texture(7, TextureSlot::BaseColor, Some(texture));
        assert!(!edit.dirty);
    }
}

/// Pixels from `p` to the segment `a`–`b`.
fn distance_to_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length2 = dx * dx + dy * dy;
    let t = if length2 > 1e-9 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / length2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (cx, cy) = (a.0 + dx * t, a.1 + dy * t);
    ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt()
}
