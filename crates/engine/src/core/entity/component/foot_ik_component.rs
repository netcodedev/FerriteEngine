//! Foot placement: after the animation has posed a model, its feet are put
//! on the ground under them with two-bone IK, the pelvis lowered so the lower
//! foot can reach, and the feet tilted to the slope.
//!
//! The animation is made for flat ground at `floor_offset` below the
//! entity's origin; each foot keeps its height above that floor, measured
//! from the ground found under it instead.

use cgmath::{
    InnerSpace, Matrix4, Point3, Quaternion, Rad, Rotation3, Transform, Vector3,
};
use glfw::{Glfw, WindowEvent};

use crate::core::{
    entity::Entity,
    model::{
        ik::{IkChain, Skeleton},
        Model,
    },
    physics::rigidbody::RigidBody,
    renderer::{
        hdr,
        line::{Line, LineRenderer},
    },
    scene::Scene,
};

use super::{model_component::ModelComponent, Component, Property, PropertyValue};

/// Ground this much above the animation's floor is still reached for.
const DEFAULT_STEP_HEIGHT: f32 = 0.6;
/// And this much below it.
const DEFAULT_STEP_DOWN: f32 = 0.5;
/// The pelvis goes down at most this far for the lower foot.
const DEFAULT_MAX_PELVIS_DROP: f32 = 0.4;
/// How fast offsets follow the ground (1/s); higher is snappier.
const DEFAULT_SMOOTHING: f32 = 14.0;
/// Feet tilt with the ground up to this angle.
const MAX_FOOT_TILT: f32 = 0.6;
/// A foot this far above its planted height no longer tilts with the
/// ground (it is in the air during a step).
const TILT_FADE_HEIGHT: f32 = 0.12;
/// Without ground this far below the floor the body counts as airborne and
/// the feet are left as animated.
const AIRBORNE_GAP: f32 = 0.35;
/// A foot this far above its planted height is swinging: it also looks for
/// ground where it is going, so it clears a step before it is over it.
const SWING_LIFT: f32 = 0.03;
/// How far ahead of its ankle (seconds of the body's movement) a swinging
/// foot looks, and at most how far (metres): less than a stair's tread, so
/// it never reaches for the step after the next one.
const LOOKAHEAD: f32 = 0.12;
const MAX_LOOKAHEAD: f32 = 0.15;
/// Ground under the ball of the foot counts only this much higher than
/// under the ankle (a bump); higher is the next step up, which the toes
/// only point at.
const TOE_STEP: f32 = 0.06;
/// Offsets rise this much faster than they sink, so a foot does not catch
/// on the edge of a step it goes up.
const RISE_SPEEDUP: f32 = 2.5;

/// One ray down, kept for the debug view.
#[derive(Clone)]
struct Probe {
    start: Point3<f32>,
    end: Point3<f32>,
    hit: Option<(Point3<f32>, Vector3<f32>)>,
}

/// What one foot found this frame, kept for the debug view.
#[derive(Clone)]
struct FootDebug {
    probes: Vec<Probe>,
    /// The highest ground the probes found, which the foot goes on.
    ground: Option<(Point3<f32>, Vector3<f32>)>,
    animated: Point3<f32>,
    target: Point3<f32>,
}

#[derive(Clone)]
struct Debug {
    feet: Vec<FootDebug>,
    floor: Point3<f32>,
    pelvis_from: Point3<f32>,
    pelvis_to: Point3<f32>,
    /// (parent, joint) world positions of the final pose.
    bones: Vec<(Point3<f32>, Point3<f32>)>,
}

impl Debug {
    fn at(floor: Point3<f32>) -> Self {
        Self {
            feet: Vec::new(),
            floor,
            pelvis_from: floor,
            pelvis_to: floor,
            bones: Vec::new(),
        }
    }
}

/// One leg's state between frames.
struct Foot {
    chain: IkChain,
    /// The ball of the foot, probed besides the ankle.
    toe: Option<usize>,
    /// Smoothed: how much higher the ground under the foot is than the
    /// animation's floor.
    offset: f32,
    /// Smoothed ground normal under the foot.
    normal: Vector3<f32>,
}

pub struct FootIk {
    pub enabled: bool,
    pub show_debug: bool,
    /// The animation's floor, relative to the entity's origin.
    pub floor_offset: f32,
    pub step_height: f32,
    pub step_down: f32,
    pub max_pelvis_drop: f32,
    pub smoothing: f32,
    /// Colliders with these tags (`user_data`) are not stood on, e.g. the
    /// slab a game puts under stairs for the body to slide up, when the
    /// steps themselves have colliders of their own.
    pub ignore_tags: Vec<u128>,
    feet: Vec<Foot>,
    /// Joint count the chains were found for; a reloaded model with
    /// another skeleton finds them again.
    joint_count: usize,
    /// Height of the ankle above the sole, in the scene's units.
    ankle_height: f32,
    pelvis: f32,
    weight: f32,
    /// Joints IK changed last frame: (joint, as animated, as IK left it).
    /// Bones a clip has no keys for keep what they are given, so IK would
    /// build on its own result frame after frame; those are put back.
    written: Vec<(usize, Matrix4<f32>, Matrix4<f32>)>,
    debug: Debug,
}

impl FootIk {
    /// Foot placement for `model` (in its file's pose), whose animation's
    /// floor is `floor_offset` below the entity's origin.
    pub fn new(model: &Model, floor_offset: f32) -> Self {
        let mut ik = Self {
            enabled: true,
            show_debug: false,
            floor_offset,
            step_height: DEFAULT_STEP_HEIGHT,
            step_down: DEFAULT_STEP_DOWN,
            max_pelvis_drop: DEFAULT_MAX_PELVIS_DROP,
            smoothing: DEFAULT_SMOOTHING,
            ignore_tags: Vec::new(),
            feet: Vec::new(),
            joint_count: 0,
            ankle_height: 0.08,
            pelvis: 0.0,
            weight: 0.0,
            written: Vec::new(),
            debug: Debug::at(Point3::new(0.0, 0.0, 0.0)),
        };
        if let Some(skeleton) = model.skeleton() {
            ik.find_feet(&skeleton);
            let positions = skeleton.positions();
            let heights: Vec<f32> = ik
                .feet
                .iter()
                .map(|foot| positions[foot.chain.end].y * model.scale())
                .collect();
            if let (Some(lowest), false) = (model.lowest_vertex(), heights.is_empty()) {
                let mean = heights.iter().sum::<f32>() / heights.len() as f32;
                ik.ankle_height = (mean - lowest).max(0.0);
            }
        }
        ik
    }

    fn find_feet(&mut self, skeleton: &Skeleton) {
        self.joint_count = skeleton.joints.len();
        self.feet = skeleton
            .foot_chains()
            .into_iter()
            .map(|chain| Foot {
                toe: skeleton.toe_of(chain.end),
                chain,
                offset: 0.0,
                normal: Vector3::unit_y(),
            })
            .collect();
    }

    fn place_feet(&mut self, scene: &mut Scene, entity: &mut Entity, dt: f32) {
        let body = entity.get_component::<RigidBody>().map(|rb| rb.rigid_body_handle);
        let ignore: Vec<_> = body.into_iter().collect();
        // Where the body is going, for swinging feet to look ahead.
        let velocity = body.map_or(Vector3::new(0.0, 0.0, 0.0), |handle| {
            let v = scene.physics_engine.rigid_bodies[handle].linvel();
            Vector3::new(v.x, 0.0, v.z)
        });
        let entity_transform = entity.local_transform();
        let entity_rotation = entity.get_rotation();
        let origin = entity.get_position();
        let Some(model) = entity
            .get_component_mut::<ModelComponent>()
            .map(|mc| mc.get_model_mut())
        else {
            return;
        };
        let Some(mut skeleton) = model.skeleton() else {
            return;
        };
        if skeleton.joints.len() != self.joint_count {
            self.find_feet(&skeleton);
            self.written.clear();
        }
        for (joint, animated, ik) in self.written.drain(..) {
            if skeleton.joints[joint].local == ik {
                skeleton.joints[joint].local = animated;
            }
        }
        let before = skeleton.locals();
        if self.feet.is_empty() {
            return;
        }
        // The model is drawn without the root motion the animation wrote
        // into its position (the game takes it as movement).
        let root = entity_transform * Matrix4::from_scale(model.scale());
        let Some(to_skeleton) = root.inverse_transform() else {
            return;
        };
        let blend = 1.0 - (-self.smoothing * dt).exp();
        let floor_y = origin.y + self.floor_offset;
        let animated = skeleton.world_positions(&root);

        // Off the ground: let go of the feet.
        let grounded = scene
            .physics_engine
            .cast_ray(
                [origin.x, origin.y, origin.z],
                [0.0, -1.0, 0.0],
                -self.floor_offset + AIRBORNE_GAP,
                &ignore,
            )
            .is_some();
        let wanted_weight = if self.enabled && grounded { 1.0 } else { 0.0 };
        self.weight += (wanted_weight - self.weight) * blend;

        let mut debug = Debug::at(Point3::new(origin.x, floor_y, origin.z));

        // Ground under each foot: the ankle's, the ball's where only a
        // little higher, and for a swinging foot what is ahead of the ankle
        // where higher, so it clears the next step's edge.
        let blend_up = 1.0 - (-self.smoothing * RISE_SPEEDUP * dt).exp();
        let reach = self.step_height + self.step_down;
        let ignore_tags = &self.ignore_tags;
        for foot in &mut self.feet {
            let at = animated[foot.chain.end];
            let lift = at.y - floor_y - self.ankle_height;
            let ahead = (lift > SWING_LIFT).then(|| {
                let ahead = velocity * LOOKAHEAD;
                if ahead.magnitude() > MAX_LOOKAHEAD {
                    ahead.normalize() * MAX_LOOKAHEAD
                } else {
                    ahead
                }
            });
            let mut probes = Vec::new();
            let mut probe = |p: Point3<f32>| {
                let start = Point3::new(p.x, floor_y + self.step_height, p.z);
                let hit = scene
                    .physics_engine
                    .cast_ray_where(
                        [start.x, start.y, start.z],
                        [0.0, -1.0, 0.0],
                        reach,
                        |_, collider| {
                            !collider.parent().is_some_and(|parent| ignore.contains(&parent))
                                && !ignore_tags.contains(&collider.user_data)
                        },
                    )
                    .map(|hit| {
                        let point = start - Vector3::unit_y() * hit.distance;
                        let normal = Vector3::from(hit.normal);
                        let normal = if normal.y > 0.2 {
                            normal.normalize()
                        } else {
                            Vector3::unit_y()
                        };
                        (point, normal)
                    });
                probes.push(Probe {
                    start,
                    end: start - Vector3::unit_y() * reach,
                    hit,
                });
                hit
            };
            let ankle = probe(at);
            let ball = foot.toe.and_then(|toe| probe(animated[toe]));
            let ahead = ahead.and_then(|ahead| probe(at + ahead));
            let mut ground = ankle.or(ball);
            if let (Some(base), Some(ball)) = (ground, ball) {
                if ball.0.y > base.0.y && ball.0.y - base.0.y <= TOE_STEP {
                    ground = Some(ball);
                }
            }
            if let Some(ahead) = ahead {
                if ground.is_none_or(|base| ahead.0.y > base.0.y) {
                    ground = Some(ahead);
                }
            }
            let (wanted_offset, wanted_normal) = match ground {
                Some((point, normal)) => (point.y - floor_y, normal),
                None => (0.0, Vector3::unit_y()),
            };
            let rate = if wanted_offset > foot.offset { blend_up } else { blend };
            foot.offset += (wanted_offset - foot.offset) * rate;
            foot.normal = (foot.normal + (wanted_normal - foot.normal) * blend).normalize();
            debug.feet.push(FootDebug {
                probes,
                ground,
                animated: at,
                target: at,
            });
        }

        // The pelvis goes down as far as the lowest foot needs.
        let lowest = self
            .feet
            .iter()
            .map(|foot| foot.offset)
            .fold(0.0f32, f32::min);
        let wanted_pelvis = lowest.max(-self.max_pelvis_drop) * self.weight;
        self.pelvis += (wanted_pelvis - self.pelvis) * blend;
        let root_joint = 0;
        let hips_before = animated[root_joint];
        if self.pelvis.abs() > 1e-5 {
            let shift = to_skeleton.transform_vector(Vector3::new(0.0, self.pelvis, 0.0));
            let joint = &mut skeleton.joints[root_joint];
            joint.local = Matrix4::from_translation(shift) * joint.local;
        }
        debug.pelvis_from = hips_before;
        debug.pelvis_to = hips_before + Vector3::new(0.0, self.pelvis, 0.0);

        // Each foot keeps its height above the floor, now above the ground
        // under it, and lies on it while planted.
        let inverse_rotation = entity_rotation.conjugate();
        for (foot, foot_debug) in self.feet.iter().zip(debug.feet.iter_mut()) {
            let at = animated[foot.chain.end];
            let target = at + Vector3::new(0.0, foot.offset * self.weight, 0.0);
            foot_debug.target = target;
            if self.weight < 1e-3 {
                continue;
            }
            skeleton.solve(&foot.chain, to_skeleton.transform_point(target));

            let lift = at.y - floor_y - self.ankle_height;
            let planted = (1.0 - lift / TILT_FADE_HEIGHT).clamp(0.0, 1.0) * self.weight;
            let axis = Vector3::unit_y().cross(foot.normal);
            let angle = foot.normal.y.clamp(-1.0, 1.0).acos().min(MAX_FOOT_TILT) * planted;
            if axis.magnitude2() > 1e-10 && angle > 1e-4 {
                let world = Quaternion::from_axis_angle(axis.normalize(), Rad(angle));
                // The same turn, in the skeleton's space (the entity only
                // turns about the vertical and scales uniformly).
                skeleton.turn(foot.chain.end, inverse_rotation * world * entity_rotation);
            }
        }

        model.apply_skeleton(&skeleton);
        self.written = skeleton
            .joints
            .iter()
            .enumerate()
            .filter(|(i, joint)| joint.local != before[*i])
            .map(|(i, joint)| (i, before[i], joint.local))
            .collect();
        if self.show_debug {
            let positions = skeleton.world_positions(&root);
            debug.bones = skeleton
                .joints
                .iter()
                .enumerate()
                .filter_map(|(i, joint)| Some((positions[joint.parent?], positions[i])))
                .collect();
        }
        self.debug = debug;
    }
}

fn segment(from: Point3<f32>, to: Point3<f32>) -> Option<Line> {
    let d = to - from;
    let length = d.magnitude();
    (length > 1e-6).then(|| Line::new(from, d / length, length))
}

fn cross(at: Point3<f32>, size: f32) -> Vec<Line> {
    [Vector3::unit_x(), Vector3::unit_y(), Vector3::unit_z()]
        .into_iter()
        .filter_map(|axis| segment(at - axis * size, at + axis * size))
        .collect()
}

fn draw(vp: &Matrix4<f32>, lines: Vec<Line>, color: [f32; 3]) {
    if !lines.is_empty() {
        LineRenderer::render_lines(vp, &lines, Vector3::from(color), true);
    }
}

impl Component for FootIk {
    fn update(&mut self, scene: &mut Scene, entity: &mut Entity, delta_time: f64) {
        self.place_feet(scene, entity, delta_time as f32);
    }

    fn update_in_editor(&mut self, scene: &mut Scene, entity: &mut Entity, delta_time: f64) {
        self.place_feet(scene, entity, delta_time as f32);
    }

    fn render_transparent(
        &self,
        _scene: &Scene,
        _entity: &Entity,
        view_projection: &Matrix4<f32>,
        _parent_transform: &Matrix4<f32>,
    ) {
        if !self.show_debug || !hdr::draws_to_view() {
            return;
        }
        let d = &self.debug;
        let vp = view_projection;

        // Bones of the final pose.
        draw(
            vp,
            d.bones.iter().filter_map(|(a, b)| segment(*a, *b)).collect(),
            [0.35, 0.6, 1.0],
        );

        // The animation's floor: a square under the body.
        let s = 0.6;
        let f = d.floor;
        let corners = [
            f + Vector3::new(-s, 0.0, -s),
            f + Vector3::new(s, 0.0, -s),
            f + Vector3::new(s, 0.0, s),
            f + Vector3::new(-s, 0.0, s),
        ];
        draw(
            vp,
            (0..4)
                .filter_map(|i| segment(corners[i], corners[(i + 1) % 4]))
                .collect(),
            [0.5, 0.5, 0.5],
        );

        // Pelvis drop.
        let mut pelvis = cross(d.pelvis_to, 0.05);
        pelvis.extend(segment(d.pelvis_from, d.pelvis_to));
        draw(vp, pelvis, [1.0, 0.55, 0.1]);

        for foot in &d.feet {
            // Rays: green to where they found ground, red where they did
            // not; the ground the foot goes on is marked with its normal.
            for probe in &foot.probes {
                match probe.hit {
                    Some((point, _)) => {
                        draw(vp, segment(probe.start, point).into_iter().collect(), [0.2, 1.0, 0.3]);
                        draw(vp, segment(point, probe.end).into_iter().collect(), [0.15, 0.35, 0.15]);
                        draw(vp, cross(point, 0.025), [0.2, 1.0, 0.3]);
                    }
                    None => draw(
                        vp,
                        segment(probe.start, probe.end).into_iter().collect(),
                        [1.0, 0.2, 0.2],
                    ),
                }
            }
            if let Some((point, normal)) = foot.ground {
                let mut ground = cross(point, 0.05);
                ground.extend(segment(point, point + normal * 0.25));
                draw(vp, ground, [0.2, 0.9, 1.0]);
            }
            // Where the animation put the foot, and where IK moved it.
            draw(vp, cross(foot.animated, 0.035), [0.4, 0.4, 1.0]);
            let mut target = cross(foot.target, 0.06);
            target.extend(segment(foot.animated, foot.target));
            draw(vp, target, [1.0, 0.2, 1.0]);
        }
    }

    fn handle_event(&mut self, _: &mut Glfw, _: &mut glfw::Window, _: &WindowEvent) {}

    fn display_name(&self) -> String {
        "Foot IK".into()
    }

    fn properties(&self) -> Vec<Property> {
        vec![
            Property::bool("enabled", self.enabled),
            Property::bool("show debug", self.show_debug),
            Property::ranged("step height", self.step_height, 0.0, 1.0),
            Property::ranged("step down", self.step_down, 0.0, 1.0),
            Property::ranged("max pelvis drop", self.max_pelvis_drop, 0.0, 1.0),
            Property::ranged("smoothing", self.smoothing, 1.0, 40.0),
            Property::text(
                "feet",
                self.feet
                    .iter()
                    .map(|foot| foot.chain.label.clone())
                    .collect::<Vec<_>>()
                    .join(", "),
            )
            .read_only(),
            Property::text("blend", format!("{:.2}", self.weight)).read_only(),
        ]
    }

    fn set_property(&mut self, name: &str, value: PropertyValue) {
        match name {
            "enabled" => self.enabled = value.as_bool().unwrap_or(self.enabled),
            "show debug" => self.show_debug = value.as_bool().unwrap_or(self.show_debug),
            "step height" => self.step_height = value.as_f32().unwrap_or(self.step_height),
            "step down" => self.step_down = value.as_f32().unwrap_or(self.step_down),
            "max pelvis drop" => {
                self.max_pelvis_drop = value.as_f32().unwrap_or(self.max_pelvis_drop)
            }
            "smoothing" => self.smoothing = value.as_f32().unwrap_or(self.smoothing).max(0.1),
            _ => {}
        }
    }
}

