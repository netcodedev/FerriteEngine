//! A clip's references in the preview (other models shown with it, see
//! `docs/anchors.md`) and the anchors of the clip's model and of the
//! references: drawn as axis crosses, picked, moved and turned.

use std::collections::HashSet;

use cgmath::{
    EuclideanSpace, InnerSpace, Matrix4, Point3, Quaternion, Rad, Rotation3, SquareMatrix,
    Transform, Vector3,
};
use ferrite::core::{
    assets::AssetId,
    model::{
        anchor::{self, Anchor, ReferenceLink},
        clip_file::ClipReference,
        grip,
        ik::Skeleton,
        Model,
    },
    renderer::{line::Line, pbr::Lighting},
};

use crate::editor::clip_editor::{ring_points, RING_COLORS};

use super::*;

/// A reference's model, loaded for the preview.
pub(super) struct RefModel {
    model: Model,
    /// Its bones in the rest pose, for anchors on bones.
    rest: Option<Skeleton>,
    /// `Model::root_transform` of the identity.
    root: Matrix4<f32>,
    /// Its meshes by where their node is (see `Model::part_offsets`).
    parts: Vec<(HashSet<String>, Vector3<f32>)>,
}

impl RefModel {
    pub(super) fn load(edit: &AssetEdit, id: AssetId) -> Result<RefModel, String> {
        let settings = edit
            .model_settings(id)
            .ok_or_else(|| tr!("asset_editor.preview.model_missing", id = id))?;
        let source = ferrite::core::assets::database()
            .get(id)
            .map(|entry| entry.source_path())
            .ok_or_else(|| tr!("asset_editor.preview.model_missing", id = id))?;
        let model = load_model(&source, &settings, 0)?;
        let rest = model.skeleton();
        let root = model.root_transform(&Matrix4::identity());
        let parts = if rest.is_some() { Vec::new() } else { model.part_offsets() };
        Ok(RefModel { model, rest, root, parts })
    }

    fn render(&self, opacity: f32, lighting: &Lighting, placed: &Matrix4<f32>, vp: &Matrix4<f32>) {
        if self.parts.len() <= 1 {
            self.model.render_faded(opacity, self.rest.is_none(), lighting, placed, vp);
            return;
        }
        for (meshes, at) in &self.parts {
            let transform = placed * Matrix4::from_translation(*at);
            self.model.render_faded_meshes(Some(meshes), opacity, self.rest.is_none(), lighting, &transform, vp);
        }
    }

    /// An anchor of this model in its own space (bones at rest).
    fn anchor_local(&self, anchor: Option<&Anchor>) -> Option<Matrix4<f32>> {
        anchor::anchor_in_model(self.rest.as_ref(), &self.root, anchor)
    }
}

/// An anchor as the preview shows it.
#[derive(Clone, Debug)]
pub(super) struct AnchorShown {
    pub(super) pick: AnchorPick,
    pub(super) anchor: Anchor,
    /// Its frame in world space.
    pub(super) frame: Matrix4<f32>,
    /// What it is relative to (the model's transform or its bone's frame).
    pub(super) base: Matrix4<f32>,
    /// The reference it places (it is that reference's anchor and the
    /// reference is placed by it): moving it moves the reference.
    pub(super) places: Option<usize>,
}

/// An anchor being moved or turned. Everything is worked out from the
/// state when the drag began.
pub(super) struct AnchorDrag {
    shown: AnchorShown,
    /// The anchor's frame in its model's own space, for one that places
    /// its reference.
    model_local: Option<(Matrix4<f32>, Matrix4<f32>)>,
    kind: AnchorDragKind,
    pub(super) moved: bool,
}

enum AnchorDragKind {
    /// Along one of the anchor's axes (an arrow).
    Axis {
        axis: usize,
        origin: Point3<f32>,
        direction: Vector3<f32>,
        /// Where along the axis the cursor was when the drag began.
        start: f32,
    },
    Move {
        plane_normal: Vector3<f32>,
        start_hit: Point3<f32>,
    },
    Turn {
        axis: usize,
        /// Screen angle of the cursor around the anchor last time.
        angle: f32,
        /// Turned so far, radians.
        total: f32,
        sign: f32,
    },
}

/// Size of the anchor crosses, as a part of the model's radius.
const ANCHOR_SIZE: f32 = 0.07;
const ANCHOR_RING: f32 = 0.11;
/// Length of the selected anchor's move arrows, as a part of the radius.
const ANCHOR_ARROW: f32 = 0.2;
/// Pixels from an arrow within which a press takes it.
const ARROW_PICK_RADIUS: f32 = 7.0;

/// Where along the line `origin + direction * t` (direction of unit
/// length) the ray comes closest; `None` when they run alongside.
fn along_line(origin: Point3<f32>, direction: Vector3<f32>, ray_origin: Point3<f32>, ray_dir: Vector3<f32>) -> Option<f32> {
    let w = origin - ray_origin;
    let b = direction.dot(ray_dir);
    let c = ray_dir.dot(ray_dir);
    let denominator = direction.dot(direction) * c - b * b;
    if denominator.abs() < 1e-6 {
        return None;
    }
    Some((b * ray_dir.dot(w) - c * direction.dot(w)) / denominator)
}

impl AssetPreview {
    /// Load the models of the clip's references when they changed.
    pub(super) fn update_references(&mut self) {
        let state = self.state.borrow();
        let Some(edit) = state.asset_edit.as_ref() else { return };
        let Some(clip) = edit.clip.as_ref() else {
            self.refs.clear();
            self.refs_for = None;
            return;
        };
        let wanted = (edit.meta.id, clip.references, edit.preview);
        if self.refs_for == Some(wanted) {
            return;
        }
        self.refs_for = Some(wanted);
        self.refs = clip
            .file
            .references
            .iter()
            .map(|reference| {
                RefModel::load(edit, reference.model)
                    .map_err(|e| ferrite::log::warn!("reference: {e}"))
                    .ok()
            })
            .collect();
    }

    /// Where reference `index` is drawn.
    pub(super) fn reference_placed(&self, index: usize) -> Option<Matrix4<f32>> {
        let Shown::Rigged(rigged) = &self.shown else { return None };
        let refm = self.refs.get(index)?.as_ref()?;
        let state = self.state.borrow();
        let edit = state.asset_edit.as_ref()?;
        let reference: &ClipReference = edit.clip.as_ref()?.file.references.get(index)?;
        let clip_model = edit.clip_model()?;
        match reference.link {
            ReferenceLink::Held(hand) => {
                let grip = edit.grip_of(reference.model)?;
                grip::held_transform(&rigged.model, &Matrix4::identity(), &grip, hand)
            }
            ReferenceLink::Follows | ReferenceLink::Carries => {
                // A name that is gone means the origin.
                let model_anchor = reference
                    .model_anchor
                    .as_deref()
                    .and_then(|name| edit.anchor_named(clip_model, name));
                let own = reference
                    .anchor
                    .as_deref()
                    .and_then(|name| edit.anchor_named(reference.model, name));
                let frame = if reference.link == ReferenceLink::Carries {
                    let mut rest = rigged.skeleton.clone();
                    rest.reset();
                    anchor::anchor_in_model(Some(&rest), &rigged.root(), model_anchor.as_ref())?
                } else {
                    anchor::anchor_in_model(Some(&rigged.skeleton), &rigged.root(), model_anchor.as_ref())?
                };
                anchor::place_on(&frame, &refm.anchor_local(own.as_ref())?)
            }
        }
    }

    /// How opaque the clip's model and each reference are drawn.
    pub(super) fn opacities(&self) -> (f32, Vec<f32>) {
        let state = self.state.borrow();
        match state.asset_edit.as_ref().and_then(|edit| edit.clip.as_ref()) {
            Some(clip) => (
                clip.file.model_opacity,
                clip.file.references.iter().map(|r| r.opacity).collect(),
            ),
            None => (1.0, Vec::new()),
        }
    }

    /// Draw the references: the opaque ones, or (`faded`) the see-through
    /// ones, which go after everything opaque.
    pub(super) fn render_references(&self, faded: bool, lighting: &Lighting, vp: &Matrix4<f32>) {
        let (_, opacities) = self.opacities();
        for (index, refm) in self.refs.iter().enumerate() {
            let Some(refm) = refm else { continue };
            let opacity = opacities.get(index).copied().unwrap_or(1.0);
            if (opacity < 0.999) != faded {
                continue;
            }
            if let Some(placed) = self.reference_placed(index) {
                refm.render(opacity, lighting, &placed, vp);
            }
        }
    }

    /// Every anchor shown: the clip model's (as posed) and the
    /// references'.
    pub(super) fn shown_anchors(&self) -> Vec<AnchorShown> {
        let Shown::Rigged(rigged) = &self.shown else { return Vec::new() };
        let (clip_model, references, mut anchors_of) = {
            let state = self.state.borrow();
            let Some(edit) = state.asset_edit.as_ref() else { return Vec::new() };
            let Some(clip) = edit.clip.as_ref() else { return Vec::new() };
            let Some(clip_model) = edit.clip_model() else { return Vec::new() };
            let mut anchors_of: Vec<(AssetId, Vec<Anchor>)> = vec![(clip_model, edit.anchors(clip_model))];
            for reference in &clip.file.references {
                anchors_of.push((reference.model, edit.anchors(reference.model)));
            }
            (clip_model, clip.file.references.clone(), anchors_of)
        };
        let mut out = Vec::new();
        let root = rigged.root();
        let (_, own) = anchors_of.remove(0);
        for anchor in own {
            let Some(base) = anchor::anchor_base(Some(&rigged.skeleton), &root, &Matrix4::identity(), anchor.bone.as_deref()) else {
                continue;
            };
            out.push(AnchorShown {
                pick: AnchorPick { asset: clip_model, name: anchor.name.clone() },
                frame: base * anchor.local(),
                base,
                anchor,
                places: None,
            });
        }
        for (index, ((asset, anchors), reference)) in anchors_of.into_iter().zip(&references).enumerate() {
            let (Some(Some(refm)), Some(placed)) = (self.refs.get(index), self.reference_placed(index)) else {
                continue;
            };
            for anchor in anchors {
                let Some(base) = anchor::anchor_base(refm.rest.as_ref(), &(placed * refm.root), &placed, anchor.bone.as_deref()) else {
                    continue;
                };
                let places = (reference.link.uses_anchors()
                    && reference.anchor.as_deref() == Some(anchor.name.as_str()))
                .then_some(index);
                out.push(AnchorShown {
                    pick: AnchorPick { asset, name: anchor.name.clone() },
                    frame: base * anchor.local(),
                    base,
                    anchor,
                    places,
                });
            }
        }
        out
    }

    fn selected_anchor(&self) -> Option<AnchorPick> {
        self.state.borrow().asset_edit.as_ref()?.anchor.clone()
    }

    fn anchor_scale(rigged: &Rigged) -> f32 {
        rigged.radius * ANCHOR_SIZE
    }

    /// The anchor whose centre is nearest to a window position.
    pub(super) fn anchor_at(&self, view: &Rect, x: f32, y: f32) -> Option<AnchorShown> {
        let Shown::Rigged(rigged) = &self.shown else { return None };
        let vp = self.view_projection(view, rigged.center, rigged.radius);
        self.shown_anchors()
            .into_iter()
            .filter_map(|shown| {
                let (sx, sy) = project(&vp, view, anchor::frame_axes(&shown.frame).0)?;
                let d = ((sx - x).powi(2) + (sy - y).powi(2)).sqrt();
                (d <= HANDLE_PICK_RADIUS).then_some((shown, d))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(shown, _)| shown)
    }

    /// The ring of the selected anchor under a window position.
    pub(super) fn anchor_ring_at(&self, view: &Rect, x: f32, y: f32) -> Option<(AnchorShown, usize)> {
        let Shown::Rigged(rigged) = &self.shown else { return None };
        let pick = self.selected_anchor()?;
        let shown = self.shown_anchors().into_iter().find(|s| s.pick == pick)?;
        let vp = self.view_projection(view, rigged.center, rigged.radius);
        let (center, axes) = anchor::frame_axes(&shown.frame);
        let radius = rigged.radius * ANCHOR_RING;
        axes.iter()
            .enumerate()
            .filter_map(|(axis, dir)| {
                let points: Vec<(f32, f32)> = ring_points(center, dir.normalize(), radius)
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
            .map(|(axis, _)| (shown.clone(), axis))
    }

    /// The move arrow of the selected anchor under a window position.
    pub(super) fn anchor_arrow_at(&self, view: &Rect, x: f32, y: f32) -> Option<(AnchorShown, usize)> {
        let Shown::Rigged(rigged) = &self.shown else { return None };
        let pick = self.selected_anchor()?;
        let shown = self.shown_anchors().into_iter().find(|s| s.pick == pick)?;
        let vp = self.view_projection(view, rigged.center, rigged.radius);
        let (origin, axes) = anchor::frame_axes(&shown.frame);
        let length = rigged.radius * ANCHOR_ARROW;
        axes.iter()
            .enumerate()
            .filter_map(|(axis, dir)| {
                // The part beyond the cross, so the diamond stays free.
                let from = project(&vp, view, origin + dir.normalize() * length * 0.35)?;
                let to = project(&vp, view, origin + dir.normalize() * length)?;
                let d = distance_to_segment((x, y), from, to);
                (d <= ARROW_PICK_RADIUS).then_some((axis, d))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(axis, _)| (shown, axis))
    }

    /// The nearest reference whose model is under a window position (one
    /// placed by anchors), and where the cursor meets it.
    pub(super) fn reference_at(&self, view: &Rect, x: f32, y: f32) -> Option<(usize, Point3<f32>)> {
        let Shown::Rigged(rigged) = &self.shown else { return None };
        let (origin, dir) = self.cursor_ray(view, rigged.center, rigged.radius, x, y)?;
        let links: Vec<ReferenceLink> = {
            let state = self.state.borrow();
            let clip = state.asset_edit.as_ref()?.clip.as_ref()?;
            clip.file.references.iter().map(|r| r.link).collect()
        };
        self.refs
            .iter()
            .enumerate()
            .filter(|(index, _)| links.get(*index).is_some_and(|l| l.uses_anchors()))
            .filter_map(|(index, refm)| {
                let refm = refm.as_ref()?;
                let placed = self.reference_placed(index)?;
                let distance = refm.model.raycast(&placed, origin, dir)?;
                Some((index, distance))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(index, distance)| (index, origin + dir * distance))
    }

    /// A click on a reference: select the anchor it is placed by, so its
    /// arrows and rings show and the model can be dragged. One placed by
    /// its origin gets an anchor there first (`attach`).
    pub(super) fn click_reference(&mut self, index: usize) {
        let mut state = self.state.borrow_mut();
        let Some(edit) = state.asset_edit.as_mut() else { return };
        let Some(reference) = edit.clip.as_ref().and_then(|clip| clip.file.references.get(index)).cloned() else {
            return;
        };
        if !reference.link.uses_anchors() {
            return;
        }
        let existing = reference
            .anchor
            .as_deref()
            .filter(|name| edit.anchor_named(reference.model, name).is_some())
            .map(str::to_string);
        let name = match existing {
            Some(name) => name,
            None => {
                let name = anchor::free_name(&edit.anchors(reference.model), "attach");
                let mut anchors = edit.anchors(reference.model);
                anchors.push(Anchor::new(&name, None, [0.0; 3]));
                edit.set_anchors(reference.model, anchors);
                let mut linked = reference.clone();
                linked.anchor = Some(name.clone());
                if let Some(clip) = edit.clip.as_mut() {
                    clip.set_reference(index, linked);
                }
                state.set_status(tr!("asset_editor.preview.gave_anchor", name = name));
                let Some(edit) = state.asset_edit.as_mut() else { return };
                edit.structure += 1;
                name
            }
        };
        let Some(edit) = state.asset_edit.as_mut() else { return };
        if let Some(clip) = edit.clip.as_mut() {
            clip.select(None);
        }
        edit.select_anchor(Some(AnchorPick { asset: reference.model, name }));
    }

    /// Select an anchor (or none); the bone selection goes, so one set of
    /// rings shows.
    pub(super) fn select_anchor(&mut self, pick: Option<AnchorPick>) {
        if pick.is_some() {
            self.with_clip(|clip| clip.select(None));
        }
        if let Some(edit) = self.state.borrow_mut().asset_edit.as_mut() {
            edit.select_anchor(pick);
        }
    }

    /// Start turning the selected anchor by a ring, or picking and moving
    /// an anchor, if the cursor is on one. With `meshes`, also a reference
    /// whose anchor is selected, grabbed by its model: asked for only after
    /// the bones' handles, rings and joints, which lie inside it.
    pub(super) fn begin_anchor_drag(&mut self, x: f32, y: f32, meshes: bool) -> bool {
        if !self.clip_mode || !self.show_skeleton.read() {
            return false;
        }
        let view = self.view_rect();
        let Shown::Rigged(rigged) = &self.shown else { return false };
        let (center, radius) = (rigged.center, rigged.radius);
        let eye = self.eye(center, radius);
        if let Some((shown, axis)) = self.anchor_arrow_at(&view, x, y) {
            let (origin, axes) = anchor::frame_axes(&shown.frame);
            let direction = axes[axis].normalize();
            let Some((ray_origin, dir)) = self.cursor_ray(&view, center, radius, x, y) else { return false };
            let Some(start) = along_line(origin, direction, ray_origin, dir) else { return false };
            let model_local = self.model_local(&shown);
            self.anchor_drag = Some(AnchorDrag {
                shown,
                model_local,
                kind: AnchorDragKind::Axis { axis, origin, direction, start },
                moved: false,
            });
            return true;
        }
        if let Some((shown, axis)) = self.anchor_ring_at(&view, x, y) {
            let vp = self.view_projection(&view, center, radius);
            let (origin, axes) = anchor::frame_axes(&shown.frame);
            let Some((cx, cy)) = project(&vp, &view, origin) else { return false };
            let sign = if axes[axis].dot(eye - origin) >= 0.0 { 1.0 } else { -1.0 };
            let model_local = self.model_local(&shown);
            self.anchor_drag = Some(AnchorDrag {
                shown,
                model_local,
                kind: AnchorDragKind::Turn {
                    axis,
                    angle: (-(y - cy)).atan2(x - cx),
                    total: 0.0,
                    sign,
                },
                moved: false,
            });
            return true;
        }
        // An anchor's diamond, or a reference whose anchor is selected
        // (grabbed anywhere on its model, with Shift).
        let (shown, origin) = match self.anchor_at(&view, x, y) {
            Some(shown) => {
                let origin = anchor::frame_axes(&shown.frame).0;
                (shown, origin)
            }
            None if !meshes => return false,
            None => {
                let Some((index, hit)) = self.reference_at(&view, x, y) else { return false };
                let selected = self.selected_anchor();
                let Some(shown) = self
                    .shown_anchors()
                    .into_iter()
                    .find(|s| s.places == Some(index) && Some(&s.pick) == selected.as_ref())
                else {
                    return false;
                };
                (shown, hit)
            }
        };
        let Some((ray_origin, dir)) = self.cursor_ray(&view, center, radius, x, y) else { return false };
        let normal = (center - eye).normalize();
        let Some(hit) = ray_plane(ray_origin, dir, origin, normal) else { return false };
        self.select_anchor(Some(shown.pick.clone()));
        let model_local = self.model_local(&shown);
        self.anchor_drag = Some(AnchorDrag {
            shown,
            model_local,
            kind: AnchorDragKind::Move {
                plane_normal: normal,
                start_hit: hit,
            },
            moved: false,
        });
        true
    }

    /// For an anchor that places its reference: its frame in the
    /// reference's own space, and the frame its bone (or the model) is in
    /// there.
    fn model_local(&self, shown: &AnchorShown) -> Option<(Matrix4<f32>, Matrix4<f32>)> {
        let refm = self.refs.get(shown.places?)?.as_ref()?;
        let base = anchor::anchor_base(refm.rest.as_ref(), &refm.root, &Matrix4::identity(), shown.anchor.bone.as_deref())?;
        Some((base * shown.anchor.local(), base))
    }

    /// Move or turn the dragged anchor as the cursor goes.
    pub(super) fn continue_anchor_drag(&mut self, x: f32, y: f32) {
        let view = self.view_rect();
        let Shown::Rigged(rigged) = &self.shown else { return };
        let (center, radius) = (rigged.center, rigged.radius);
        let vp = self.view_projection(&view, center, radius);
        let ray = self.cursor_ray(&view, center, radius, x, y);
        let Some(drag) = &mut self.anchor_drag else { return };
        let (origin, axes) = anchor::frame_axes(&drag.shown.frame);
        // What the drag does to the anchor, in world space.
        let delta = match &mut drag.kind {
            AnchorDragKind::Axis { origin, direction, start, .. } => {
                let Some((ray_origin, dir)) = ray else { return };
                let Some(t) = along_line(*origin, *direction, ray_origin, dir) else { return };
                Matrix4::from_translation(*direction * (t - *start))
            }
            AnchorDragKind::Move { plane_normal, start_hit } => {
                let Some((ray_origin, dir)) = ray else { return };
                let Some(hit) = ray_plane(ray_origin, dir, *start_hit, *plane_normal) else { return };
                Matrix4::from_translation(hit - *start_hit)
            }
            AnchorDragKind::Turn { axis, angle, total, sign } => {
                let Some((cx, cy)) = project(&vp, &view, origin) else { return };
                let now = (-(y - cy)).atan2(x - cx);
                let mut step = now - *angle;
                while step > std::f32::consts::PI {
                    step -= std::f32::consts::TAU;
                }
                while step < -std::f32::consts::PI {
                    step += std::f32::consts::TAU;
                }
                *angle = now;
                *total += step;
                let turn = Quaternion::from_axis_angle(axes[*axis].normalize(), Rad(*total * *sign));
                Matrix4::from_translation(origin.to_vec())
                    * Matrix4::from(turn)
                    * Matrix4::from_translation(-origin.to_vec())
            }
        };
        drag.moved = true;
        let found = match drag.model_local {
            // The anchor places its reference: the reference moves with
            // the cursor, so the anchor goes the other way on it.
            Some((local, base)) => anchor::moved_placing(&drag.shown.frame, &local, &base, &delta),
            None => anchor::moved(&drag.shown.base, &drag.shown.frame, &delta),
        };
        let Some((position, rotation)) = found else { return };
        let mut anchor = drag.shown.anchor.clone();
        anchor.position = position.map(rounded);
        anchor.set_rotation(&rotation);
        let pick = drag.shown.pick.clone();
        if let Some(edit) = self.state.borrow_mut().asset_edit.as_mut() {
            edit.set_anchor(pick.asset, &pick.name, anchor);
        }
    }

    /// The drag ended: the inspector shows the new values.
    pub(super) fn finish_anchor_drag(&mut self) -> bool {
        let Some(drag) = self.anchor_drag.take() else { return false };
        if drag.moved {
            if let Some(edit) = self.state.borrow_mut().asset_edit.as_mut() {
                edit.structure += 1;
            }
        }
        true
    }

    /// Draw the anchors: an axis cross each, the selected one with rings.
    /// And where the other hand goes on a model held in both hands (a
    /// blue diamond, to pose that hand to).
    pub(super) fn render_anchors(&self, rigged: &Rigged, vp: &Matrix4<f32>, view: &Rect) {
        let size = Self::anchor_scale(rigged);
        let held: Vec<(usize, [f32; 3])> = {
            let state = self.state.borrow();
            state
                .asset_edit
                .as_ref()
                .and_then(|edit| {
                    let clip = edit.clip.as_ref()?;
                    Some(
                        clip.file
                            .references
                            .iter()
                            .enumerate()
                            .filter(|(_, r)| matches!(r.link, ReferenceLink::Held(_)))
                            .filter_map(|(i, r)| Some((i, edit.grip_of(r.model)?.other_hand?)))
                            .collect(),
                    )
                })
                .unwrap_or_default()
        };
        for (index, point) in held {
            if let Some(placed) = self.reference_placed(index) {
                let at = placed.transform_point(Point3::from(point));
                render_thick_lines(vp, &diamond(at, size * 0.3), BONE_COLOR, view, 2);
            }
        }
        let selected = self.selected_anchor();
        let hot_ring = match &self.anchor_drag {
            Some(AnchorDrag { kind: AnchorDragKind::Turn { axis, .. }, .. }) => Some(*axis),
            _ => self.hovered_anchor_ring,
        };
        let hot_arrow = match &self.anchor_drag {
            Some(AnchorDrag { kind: AnchorDragKind::Axis { axis, .. }, .. }) => Some(*axis),
            _ => self.hovered_anchor_arrow,
        };
        for shown in self.shown_anchors() {
            let (origin, axes) = anchor::frame_axes(&shown.frame);
            for (axis, dir) in axes.iter().enumerate() {
                if let Some(line) = segment(origin, origin + dir.normalize() * size) {
                    render_thick_lines(vp, &vec![line], RING_COLORS[axis], view, 2);
                }
            }
            let is_selected = selected.as_ref() == Some(&shown.pick);
            let hovered = self.hovered_anchor.as_ref() == Some(&shown.pick);
            let color = if is_selected {
                SELECTED_COLOR
            } else if hovered {
                HANDLE_HOT_COLOR
            } else if shown.places.is_some() {
                // Where the reference hangs on.
                (1.0, 0.55, 0.85)
            } else {
                HANDLE_COLOR
            };
            render_thick_lines(vp, &diamond(origin, size * 0.3), color, view, 2);
            if is_selected {
                let length = rigged.radius * ANCHOR_ARROW;
                for (axis, dir) in axes.iter().enumerate() {
                    let dir = dir.normalize();
                    let tip = origin + dir * length;
                    // A shaft and a little head of two strokes.
                    let side = if dir.y.abs() < 0.9 { dir.cross(Vector3::unit_y()) } else { dir.cross(Vector3::unit_x()) }
                        .normalize()
                        * length
                        * 0.08;
                    let back = tip - dir * length * 0.15;
                    let lines: Vec<Line> = [
                        segment(origin + dir * length * 0.35, tip),
                        segment(tip, back + side),
                        segment(tip, back - side),
                    ]
                    .into_iter()
                    .flatten()
                    .collect();
                    let color = if Some(axis) == hot_arrow { (1.0, 0.9, 0.4) } else { RING_COLORS[axis] };
                    render_thick_lines(vp, &lines, color, view, 3);
                }
                let radius = rigged.radius * ANCHOR_RING;
                for (axis, dir) in axes.iter().enumerate() {
                    let points = ring_points(origin, dir.normalize(), radius);
                    let lines: Vec<Line> = points.windows(2).filter_map(|w| segment(w[0], w[1])).collect();
                    let color = if Some(axis) == hot_ring { (1.0, 0.9, 0.4) } else { RING_COLORS[axis] };
                    render_thick_lines(vp, &lines, color, view, 2);
                }
            }
        }
    }

    /// Track which anchor (or ring of the selected one) is under the cursor.
    pub(super) fn hover_anchors(&mut self, x: f32, y: f32, free: bool) {
        if !(free && self.clip_mode && self.show_skeleton.read()) {
            self.hovered_anchor = None;
            self.hovered_anchor_ring = None;
            self.hovered_anchor_arrow = None;
            return;
        }
        let view = self.view_rect();
        self.hovered_anchor_arrow = self.anchor_arrow_at(&view, x, y).map(|(_, axis)| axis);
        self.hovered_anchor_ring = if self.hovered_anchor_arrow.is_none() {
            self.anchor_ring_at(&view, x, y).map(|(_, axis)| axis)
        } else {
            None
        };
        self.hovered_anchor = if self.hovered_anchor_ring.is_none() && self.hovered_anchor_arrow.is_none() {
            self.anchor_at(&view, x, y).map(|shown| shown.pick)
        } else {
            None
        };
    }

    /// Draw the anchors of the asset itself outside a clip (a static
    /// model, or one with bones in its current pose): crosses only.
    pub(super) fn render_own_anchors(
        &self,
        skeleton: Option<(&Skeleton, Matrix4<f32>)>,
        size: f32,
        vp: &Matrix4<f32>,
        view: &Rect,
    ) {
        let (anchors, selected) = {
            let state = self.state.borrow();
            let Some(edit) = state.asset_edit.as_ref() else { return };
            (edit.anchors(edit.meta.id), edit.anchor.clone())
        };
        for anchor in anchors {
            let frame = anchor::anchor_frame_in(
                skeleton.map(|(s, _)| s),
                &skeleton.map_or(Matrix4::identity(), |(_, root)| root),
                &Matrix4::identity(),
                &anchor,
            );
            let Some(frame) = frame else { continue };
            let (origin, axes) = anchor::frame_axes(&frame);
            for (axis, dir) in axes.iter().enumerate() {
                if let Some(line) = segment(origin, origin + dir.normalize() * size) {
                    render_thick_lines(vp, &vec![line], RING_COLORS[axis], view, 2);
                }
            }
            let picked = selected.as_ref().is_some_and(|p| p.name == anchor.name);
            let color = if picked { SELECTED_COLOR } else { HANDLE_COLOR };
            render_thick_lines(vp, &diamond(origin, size * 0.3), color, view, 2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cursor_ray_meets_an_axis_where_it_passes_closest() {
        // The x axis through (0, 1, 0), seen from above at x = 0.3.
        let t = along_line(
            Point3::new(0.0, 1.0, 0.0),
            Vector3::unit_x(),
            Point3::new(0.3, 5.0, 0.2),
            -Vector3::unit_y(),
        )
        .unwrap();
        assert!((t - 0.3).abs() < 1e-5);
        // Looking along the axis gives nothing to go by.
        assert!(along_line(Point3::origin(), Vector3::unit_x(), Point3::new(-2.0, 0.0, 0.0), Vector3::unit_x()).is_none());
    }
}
