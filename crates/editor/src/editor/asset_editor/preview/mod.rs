//! The preview of the asset being edited, where the scene viewport normally
//! is.

use std::time::Instant;

use cgmath::{
    perspective, Deg, EuclideanSpace, InnerSpace, Matrix4, Point3, SquareMatrix, Transform,
    Vector3, Vector4,
};
use ferrite::{
    core::{
        assets::{self, AssetId, AssetKind, ModelSettings},
        model::{
            clip_file::{BonePose, Transform as BoneTransform},
            grip::Grip,
        },
        primitives::{Position, Size},
        renderer::text::{icons, Text},
        utils::DataSource,
    },
    terrain::scatter::{graph::Placement, ScatterInstances, ScatterModel, ScatterSpec},
};
use ferrite_ui::ui::{
    element_handle::UIElementHandle,
    elements::{
        button::{Button, ButtonStyle},
        image::Image,
        panel::Panel,
    },
    theme,
};

use crate::editor::{
    clip_editor::{
        ring_points, ClipEdit, ClipRequest, Timeline, TimelineAction, AXES, TIMELINE_HEIGHT,
    },
    layout::Rect,
    Shared,
};

use super::*;

mod references;
mod ui;

use references::{AnchorDrag, RefModel};

/// Seconds the settings have to stay untouched before the preview loads
/// the model again (typing a scale would otherwise reload per digit).
pub(super) const RELOAD_DELAY: f32 = 0.35;

/// Since when `wanted` has been wanted: `pending` keeps the key last seen
/// and when it came. Every new edit (a new key) starts the wait again, so
/// the reload waits for the last edit, not the first.
pub(super) fn settled_since<K: PartialEq + Copy>(
    pending: &mut Option<(Instant, K)>,
    wanted: K,
    now: Instant,
) -> Instant {
    match pending {
        Some((since, key)) if *key == wanted => *since,
        _ => {
            *pending = Some((now, wanted));
            now
        }
    }
}
/// Closest and farthest the camera gets to what it looks at, in model
/// radii (with panning, close enough to look at a hand on a wheel).
pub(super) const MIN_ZOOM: f32 = 0.03;
pub(super) const MAX_ZOOM: f32 = 12.0;
/// Camera distance in model radii at which a model fits the view.
pub(super) const DEFAULT_ZOOM: f32 = 3.6;
/// Longest edge (px) a texture is shown at, room permitting.
pub(super) const TEXTURE_SIDE: f32 = 384.0;

struct Orbit {
    yaw: f32,
    pitch: f32,
    /// In multiples of the model's radius.
    zoom: f32,
    dragging: Option<(f64, f64)>,
    /// Where the camera looks, moved from the model's centre (right or
    /// middle mouse button).
    pan: Vector3<f32>,
    panning: Option<(f64, f64)>,
}

/// What the preview currently shows.
pub(super) enum Shown {
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

/// Pixels from a joint within which a click selects it.
const JOINT_PICK_RADIUS: f32 = 8.0;
/// Pixels from a ring within which a press takes it.
const RING_PICK_RADIUS: f32 = 6.0;
pub(super) const SELECTED_COLOR: (f32, f32, f32) = (0.55, 0.95, 0.55);

/// Pixels from a handle within which a click takes it.
const HANDLE_PICK_RADIUS: f32 = 12.0;
pub(super) const BONE_COLOR: (f32, f32, f32) = (0.45, 0.75, 1.0);
pub(super) const HANDLE_COLOR: (f32, f32, f32) = (1.0, 0.8, 0.2);
pub(super) const HANDLE_HOT_COLOR: (f32, f32, f32) = (1.0, 0.4, 0.1);

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
    /// The `(asset, preview counter)` waiting to be shown, and since when
    /// it is wanted (see `settled_since`).
    pending_since: Option<(Instant, Option<(AssetId, u64)>)>,
    message: Text,
    hint: Text,
    orbit: Orbit,
    /// Whether the bones and IK handles of a rigged model are drawn.
    show_skeleton: DataSource<bool>,
    reset_pose: DataSource<bool>,
    skeleton_button: Button,
    reset_button: Button,
    frame_button: Button,
    /// Set by the frame button: the camera goes back to the model.
    frame_request: DataSource<bool>,
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
    /// The models of the clip's references (`None` where one failed to
    /// load), and the `(asset, references counter, preview counter)` they
    /// were loaded for.
    refs: Vec<Option<RefModel>>,
    refs_for: Option<(AssetId, u64, u64)>,
    anchor_drag: Option<AnchorDrag>,
    hovered_anchor: Option<AnchorPick>,
    hovered_anchor_ring: Option<usize>,
    hovered_anchor_arrow: Option<usize>,
    /// A press on a reference that orbits unless it ends where it began:
    /// then it selects the reference (`click_reference`).
    reference_press: Option<(usize, f64, f64)>,
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
        let frame_request = DataSource::new(false);
        let frame_flag = frame_request.clone();
        let mut frame_button = header_button(
            "Center view",
            icons::SCAN,
            ButtonStyle::Secondary,
            Box::new(move |_| frame_flag.write(true)),
        );
        frame_button.set_tooltip("Look at the model again (right or middle mouse button moves the view)");
        let timeline = Timeline::new(state.clone());
        Self {
            handle: UIElementHandle::new(),
            state,
            panel,
            show_skeleton,
            reset_pose,
            skeleton_button,
            reset_button,
            frame_button,
            frame_request,
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
            refs: Vec::new(),
            refs_for: None,
            anchor_drag: None,
            hovered_anchor: None,
            hovered_anchor_ring: None,
            hovered_anchor_arrow: None,
            reference_press: None,
            shown: Shown::Nothing,
            shown_for: None,
            framed_for: None,
            pending_since: None,
            message: theme::text("", theme::FONT_SIZE, theme::TEXT_MUTED),
            hint: theme::text(
                "Drag to orbit, right or middle mouse to move the view, scroll to zoom",
                theme::FONT_SIZE_SMALL,
                theme::TEXT_DIM,
            ),
            orbit: Orbit {
                yaw: 0.6,
                pitch: 0.25,
                zoom: DEFAULT_ZOOM,
                dragging: None,
                pan: Vector3::new(0.0, 0.0, 0.0),
                panning: None,
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
        // Hover is only updated on cursor moves: what it points at may not
        // exist in the new model.
        self.hovered_joint = None;
        self.hovered_ring = None;
        self.touched.clear();
        self.posed_for = None;
        self.ring_drag = None;
        self.anchor_drag = None;
        self.hovered_anchor = None;
        self.hovered_anchor_ring = None;
        self.hovered_anchor_arrow = None;
        self.reference_press = None;
        self.refs_for = None;
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
            AssetKind::ScatterGraph(_) => Shown::Note("A scatter graph. It is edited in the node editor (to come).".into()),
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
                "Drag a handle to pose, elsewhere to orbit; right or middle mouse moves the view, scroll zooms"
            }
            Shown::Held(_) => {
                "Drag the model to move it in the hand, the blue diamond for the other hand; elsewhere to orbit"
            }
            _ => "Drag to orbit, right or middle mouse to move the view, scroll to zoom",
        });
    }

    /// Show the model a clip is posed on; false if the asset is no clip.
    fn load_clip(&mut self) -> bool {
        let model = {
            let state = self.state.borrow();
            let Some(edit) = state.asset_edit.as_ref().filter(|edit| edit.clip.is_some()) else {
                return false;
            };
            let id = edit.clip_model();
            let database = assets::database();
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
            "Drag a handle or ring to pose, click a joint to select it. Click a reference or an anchor, then drag its arrows, diamond or rings, or Shift+drag the model. Drag to orbit, right or middle mouse moves the view",
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
            if name.is_some() {
                if let Some(edit) = self.state.borrow_mut().asset_edit.as_mut() {
                    edit.select_anchor(None);
                }
            }
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
                        let spec = ScatterSpec::of_model(loaded);
                        Shown::Model {
                            instances: ScatterInstances::new(&spec, &[Placement::upright([0.0; 3], 0.0)]),
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

    /// The centre and radius the camera is framed by, for what is shown.
    pub(super) fn framing(&self) -> Option<(Point3<f32>, f32)> {
        match &self.shown {
            Shown::Model { center, radius, .. } => Some((*center, *radius)),
            Shown::Rigged(rigged) => Some((rigged.center, rigged.radius)),
            Shown::Held(held) => Some((held.center, held.radius)),
            _ => None,
        }
    }

    /// Move the camera's target by a cursor move of `(dx, dy)` pixels: the
    /// model follows the cursor.
    pub(super) fn pan_by(&mut self, dx: f32, dy: f32) {
        let Some((_, radius)) = self.framing() else { return };
        let view = self.view_rect();
        let o = &self.orbit;
        let back = Vector3::new(o.pitch.cos() * o.yaw.sin(), o.pitch.sin(), o.pitch.cos() * o.yaw.cos());
        let right = (-back).cross(Vector3::unit_y()).normalize();
        let up = right.cross(-back).normalize();
        // Metres per pixel at the target's distance (38° field of view).
        let per_pixel = 2.0 * radius * o.zoom * (19.0f32).to_radians().tan() / view.h.max(1.0);
        self.orbit.pan += (-right * dx + up * dy) * per_pixel;
    }

    fn eye(&self, center: Point3<f32>, radius: f32) -> Point3<f32> {
        let center = center + self.orbit.pan;
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
            // Close to the target (zoomed in on a hand) nothing near may be
            // cut off: the near plane comes with the camera.
            (radius * self.orbit.zoom * 0.05).clamp(0.0005, radius * 0.02),
            radius * 40.0,
        );
        projection * Matrix4::look_at_rh(eye, center + self.orbit.pan, Vector3::unit_y())
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::settled_since;

    #[test]
    fn the_wait_starts_again_with_every_edit() {
        let t0 = Instant::now();
        let later = |ms| t0 + Duration::from_millis(ms);
        let mut pending = None;
        assert_eq!(settled_since(&mut pending, 1, t0), t0);
        // The same edit still waits from when it came.
        assert_eq!(settled_since(&mut pending, 1, later(200)), t0);
        // Another edit: from now.
        assert_eq!(settled_since(&mut pending, 2, later(250)), later(250));
        assert_eq!(settled_since(&mut pending, 2, later(500)), later(250));
    }
}
