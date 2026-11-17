//! Building from blueprints. The player puts down a blueprint (a
//! foundation, a wall, a wall with a door frame), which only shows as a
//! see-through outline, and then builds it part by part: stones into a
//! foundation, beams into walls, each one aimed at the very spot it goes.
//!
//! Controls:
//!
//! | Input | |
//! | --- | --- |
//! | 1, 2, 3 / mouse wheel | Choose blueprint, stones or beams in the inventory |
//! | Right click (blueprint) | Radial menu: foundation, wall, door frame |
//! | Left click (blueprint) | Put the blueprint down |
//! | Left click (stones, beams) | Put the part in where the crosshair is |
//! | R | Turn a free-standing foundation |
//! | WASD, Space, mouse | Walk, jump, look; F flies (Space/Shift up and down) |
//! | Esc | Free or catch the mouse |
//!
//! Foundations snap to the side of foundations near them, walls to the
//! edges of foundations. A part goes in only once the parts below it are
//! in, and wall parts once their foundation is finished.

use std::cell::RefCell;
use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_2, PI};
use std::sync::Arc;

use ferrite::core::{
    entity::component::camera_component::CameraComponent,
    headless,
    model::{generated, Model},
    renderer::pbr::Lighting,
};
use ferrite::glfw;
use ferrite::prelude::*;
use ferrite::terrain::{dual_contouring::DualContouringChunk, generator::WorldGenerator, Terrain};

use crate::build_hud::{radial_choice, Hud, HudState, Item};
use crate::building_parts::{
    foundation_beside, ray_slot, wall_on_edge, Blueprint, Material, Piece, PieceKind, ShapeKey,
    FOUNDATION_DEPTH, FOUNDATION_SIZE, WALL_THICKNESS,
};

/// See-through looks of the parts not built yet, `(r, g, b, opacity)`.
const GHOST: [f32; 4] = [0.35, 0.66, 1.0, 0.20];
const GHOST_AIMED: [f32; 4] = [0.60, 0.88, 1.0, 0.62];
const GHOST_BLOCKED: [f32; 4] = [1.0, 0.55, 0.22, 0.50];
const PREVIEW: [f32; 4] = [0.40, 0.78, 1.0, 0.34];
const PREVIEW_BAD: [f32; 4] = [1.0, 0.28, 0.22, 0.34];

const EYE_HEIGHT: f32 = 1.7;
const GRAVITY: f32 = 20.0;
const JUMP_SPEED: f32 = 6.5;
/// How far away a blueprint can be put down.
const PLACE_DISTANCE: f32 = 14.0;
/// How far the radial menu's cursor goes from its centre, in pixels.
const RADIAL_RADIUS: f32 = 150.0;
const RADIAL_DEAD_ZONE: f32 = 34.0;
const MESSAGE_SECONDS: f32 = 2.6;

/// What the crosshair is on.
#[derive(Clone, Debug, PartialEq)]
pub enum Aim {
    Nothing,
    /// Where the chosen blueprint would go, and why not, if it cannot.
    Place {
        kind: PieceKind,
        position: Vector3<f32>,
        yaw: f32,
        foundation: Option<usize>,
        problem: Option<&'static str>,
    },
    /// A part of a blueprint the held material fits, and why it cannot go
    /// in yet, if it cannot.
    Part {
        piece: usize,
        slot: usize,
        problem: Option<&'static str>,
    },
}

#[derive(Default, Clone, Copy)]
struct Keys {
    forward: bool,
    back: bool,
    left: bool,
    right: bool,
    up: bool,
    down: bool,
}

/// The player's building: the blueprints in the world, the inventory, and
/// walking around to build.
#[derive(Script)]
pub struct Builder {
    /// Walking speed, metres per second.
    pub walk_speed: f32,
    /// Flying speed (F), metres per second.
    pub fly_speed: f32,
    /// How far away a part can be put in, in metres.
    pub reach: f32,
    /// Degrees the view turns per pixel of mouse movement.
    pub mouse_sensitivity: f32,
    /// Where the player starts: x and z (stands on the ground there).
    pub start: [f32; 2],
    /// Which way the player looks at the start, degrees.
    pub start_yaw: f32,

    blueprints: HashMap<PieceKind, Blueprint>,
    pieces: Vec<Piece>,
    /// The meshes of the parts, made once there is a GPU. The tint is set
    /// for each drawing of a see-through part.
    models: HashMap<ShapeKey, RefCell<Model>>,
    generator: Option<Arc<WorldGenerator>>,

    selected: Item,
    blueprint: PieceKind,
    /// Extra turn of a free-standing foundation (R).
    turn: f32,
    /// The radial menu, when open: its cursor, from the centre.
    radial: Option<(f32, f32)>,
    aim: Aim,
    /// The piece under the crosshair, for its progress.
    target: Option<usize>,
    message: Option<(String, f32, bool)>,
    /// Left clicks since the last update.
    clicks: usize,

    keys: Keys,
    look: (f32, f32),
    last_cursor: Option<(f64, f64)>,
    eye: Option<Vector3<f32>>,
    yaw: f32,
    pitch: f32,
    vertical_speed: f32,
    flying: bool,
    jump: bool,
    release_cursor: bool,
    /// Set once the game runs (the HUD is not shown while editing).
    playing: bool,
    hud: Hud,
}

impl Default for Builder {
    fn default() -> Self {
        let blueprints = PieceKind::ALL.into_iter().map(|k| (k, Blueprint::of(k))).collect();
        Self {
            walk_speed: 5.0,
            fly_speed: 14.0,
            reach: 7.0,
            mouse_sensitivity: 0.12,
            start: [0.0, 0.0],
            start_yaw: 0.0,
            blueprints,
            pieces: Vec::new(),
            models: HashMap::new(),
            generator: None,
            selected: Item::Blueprint,
            blueprint: PieceKind::Foundation,
            turn: 0.0,
            radial: None,
            aim: Aim::Nothing,
            target: None,
            message: None,
            clicks: 0,
            keys: Keys::default(),
            look: (0.0, 0.0),
            last_cursor: None,
            eye: None,
            yaw: 0.0,
            pitch: 0.0,
            vertical_speed: 0.0,
            flying: false,
            jump: false,
            release_cursor: false,
            playing: false,
            hud: Hud::default(),
        }
    }
}

/// Toward where the camera looks.
fn view_direction(yaw: f32, pitch: f32) -> Vector3<f32> {
    let (sp, cp) = pitch.sin_cos();
    let (sy, cy) = yaw.sin_cos();
    Vector3::new(cp * cy, sp, cp * sy).normalize()
}

fn horizontal(v: Vector3<f32>) -> f32 {
    (v.x * v.x + v.z * v.z).sqrt()
}

impl Builder {
    // ── For other scripts (and tests): the building without the mouse ──

    /// The blueprints in the world.
    pub fn pieces(&self) -> &[Piece] {
        &self.pieces
    }

    pub fn blueprint_of(&self, kind: PieceKind) -> &Blueprint {
        &self.blueprints[&kind]
    }

    /// Choose what is in hand.
    pub fn select(&mut self, item: Item, blueprint: PieceKind) {
        self.selected = item;
        self.blueprint = blueprint;
        self.radial = None;
    }

    /// Open the radial menu with its cursor at `cursor` (from the centre),
    /// or close it.
    pub fn set_radial(&mut self, cursor: Option<(f32, f32)>) {
        self.radial = cursor;
    }

    /// Stand at `eye` looking along yaw and pitch (radians).
    pub fn set_view(&mut self, eye: Vector3<f32>, yaw: f32, pitch: f32) {
        self.eye = Some(eye);
        self.yaw = yaw;
        self.pitch = pitch;
        self.flying = true;
    }

    /// What a left click would do now.
    pub fn aim(&self) -> &Aim {
        &self.aim
    }

    /// Click, as the left mouse button does.
    pub fn click(&mut self) {
        self.clicks += 1;
    }

    /// Put a part in, if the parts it rests on are in.
    pub fn fill(&mut self, piece: usize, slot: usize) -> Result<(), &'static str> {
        if let Some(problem) = self.part_problem(piece, slot) {
            return Err(problem);
        }
        let (p, s) = self.resolve(piece, slot);
        self.pieces[p].filled[s] = true;
        Ok(())
    }

    // ── The rules ──

    /// The piece and slot a slot really is (a shared corner post is the
    /// first wall's).
    fn resolve(&self, piece: usize, slot: usize) -> (usize, usize) {
        self.pieces[piece].links[slot].unwrap_or((piece, slot))
    }

    fn is_filled(&self, piece: usize, slot: usize) -> bool {
        let (p, s) = self.resolve(piece, slot);
        self.pieces[p].filled[s]
    }

    /// Whether a part's outline is shown (and can be aimed at): it is in,
    /// or every part it rests on is in. Layer by layer, the next one
    /// appears once the one below is done.
    fn is_shown(&self, piece: usize, slot: usize) -> bool {
        let (p, s) = self.resolve(piece, slot);
        self.pieces[p].filled[s]
            || self.blueprints[&self.pieces[p].kind].supports[s]
                .iter()
                .all(|below| self.is_filled(p, *below))
    }

    /// Parts in and parts in all.
    fn progress(&self, piece: usize) -> (usize, usize) {
        let total = self.pieces[piece].filled.len();
        let done = (0..total).filter(|s| self.is_filled(piece, *s)).count();
        (done, total)
    }

    fn is_complete(&self, piece: usize) -> bool {
        let (done, total) = self.progress(piece);
        done == total
    }

    /// Why a part cannot go in yet, if it cannot.
    fn part_problem(&self, piece: usize, slot: usize) -> Option<&'static str> {
        let (p, s) = self.resolve(piece, slot);
        let kind = self.pieces[p].kind;
        if self.pieces[p].filled[s] {
            return Some("Da ist schon ein Teil");
        }
        if let Some(foundation) = self.pieces[p].foundation {
            if !self.is_complete(foundation) {
                return Some("Erst das Fundament fertig bauen");
            }
        }
        let blueprint = &self.blueprints[&kind];
        if blueprint.supports[s].iter().any(|below| !self.is_filled(p, *below)) {
            return Some(match blueprint.slots[s].material {
                Material::Stone => "Erst die Steine darunter setzen",
                Material::Beam => "Erst die Balken darunter setzen",
            });
        }
        None
    }

    /// Put a blueprint down. A wall shares its corner posts with walls
    /// already standing there.
    pub fn place(&mut self, kind: PieceKind, position: Vector3<f32>, yaw: f32, foundation: Option<usize>) -> usize {
        let blueprint = &self.blueprints[&kind];
        let mut piece = Piece::new(kind, position, yaw, blueprint);
        piece.foundation = foundation;
        if kind.is_wall() {
            for (i, slot) in blueprint.slots.iter().enumerate() {
                if !slot.corner_post {
                    continue;
                }
                let at = piece.to_world(slot.center);
                piece.links[i] = self.pieces.iter().enumerate().find_map(|(p, other)| {
                    if !other.kind.is_wall() {
                        return None;
                    }
                    self.blueprints[&other.kind].slots.iter().enumerate().find_map(|(s, o)| {
                        (o.corner_post && other.links[s].is_none() && (other.to_world(o.center) - at).magnitude() < 0.05)
                            .then_some((p, s))
                    })
                });
            }
        }
        self.pieces.push(piece);
        self.pieces.len() - 1
    }

    // ── The world ──

    /// Height of the ground (terrain) at `(x, z)`.
    fn terrain_height(&self, x: f32, z: f32) -> f32 {
        self.generator
            .as_ref()
            .map_or(0.0, |g| g.surface_height(x as f64, z as f64))
    }

    /// What the player stands on at `(x, z)`: the terrain, or a finished
    /// foundation.
    fn floor_height(&self, x: f32, z: f32) -> f32 {
        let mut floor = self.terrain_height(x, z);
        for (i, piece) in self.pieces.iter().enumerate() {
            if piece.kind != PieceKind::Foundation || !self.is_complete(i) {
                continue;
            }
            let local = piece.to_local(Vector3::new(x, piece.position.y, z));
            let half = FOUNDATION_SIZE / 2.0 + 0.2;
            if local.x.abs() < half && local.z.abs() < half {
                floor = floor.max(piece.position.y);
            }
        }
        floor
    }

    /// Where a ray meets the terrain, within `max` metres.
    fn ray_terrain(&self, origin: Vector3<f32>, dir: Vector3<f32>, max: f32) -> Option<f32> {
        self.generator.as_ref()?;
        let step = 0.25;
        let below = |t: f32| {
            let p = origin + dir * t;
            p.y < self.terrain_height(p.x, p.z)
        };
        if below(0.0) {
            return Some(0.0);
        }
        let mut t = step;
        while t <= max + step {
            if below(t) {
                let (mut lo, mut hi) = (t - step, t);
                for _ in 0..12 {
                    let mid = 0.5 * (lo + hi);
                    if below(mid) {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
                return (hi <= max).then_some(hi);
            }
            t += step;
        }
        None
    }

    /// The nearest part box a ray enters within `max`: which piece and
    /// slot, and how far. `wanted` picks the slots that count.
    fn ray_parts(
        &self,
        origin: Vector3<f32>,
        dir: Vector3<f32>,
        max: f32,
        grow: f32,
        wanted: impl Fn(usize, usize) -> bool,
    ) -> Option<(usize, usize, f32)> {
        self.ray_parts_all(origin, dir, max, grow, wanted).into_iter().next()
    }

    /// Every part box a ray enters within `max`, nearest first.
    fn ray_parts_all(
        &self,
        origin: Vector3<f32>,
        dir: Vector3<f32>,
        max: f32,
        grow: f32,
        wanted: impl Fn(usize, usize) -> bool,
    ) -> Vec<(usize, usize, f32)> {
        let mut hits = Vec::new();
        for (p, piece) in self.pieces.iter().enumerate() {
            let blueprint = &self.blueprints[&piece.kind];
            // A quick test against the whole piece first.
            let reach = if piece.kind == PieceKind::Foundation { 2.6 } else { 2.4 };
            let centre = piece.position + Vector3::new(0.0, if piece.kind.is_wall() { 1.5 } else { -0.6 }, 0.0);
            let to = centre - origin;
            let along = to.dot(dir).clamp(0.0, max);
            if (to - dir * along).magnitude() > reach + grow {
                continue;
            }
            for (s, slot) in blueprint.slots.iter().enumerate() {
                if piece.links[s].is_some() || !wanted(p, s) {
                    continue;
                }
                if let Some(t) = ray_slot(piece, slot, origin, dir, max, grow) {
                    hits.push((p, s, t));
                }
            }
        }
        hits.sort_by(|a, b| a.2.total_cmp(&b.2));
        hits
    }

    /// What the crosshair is on, for what is in hand.
    fn compute_aim(&mut self) {
        self.target = None;
        let Some(eye) = self.eye else {
            self.aim = Aim::Nothing;
            return;
        };
        let dir = view_direction(self.yaw, self.pitch);
        // Built parts block the view; the outlines do not.
        let solid = self.ray_parts(eye, dir, PLACE_DISTANCE, 0.0, |p, s| self.is_filled(p, s));
        let ground = self.ray_terrain(eye, dir, PLACE_DISTANCE);
        let any = self.ray_parts(eye, dir, PLACE_DISTANCE, 0.0, |p, s| self.is_shown(p, s));
        self.target = any.map(|(p, _, _)| p);
        match self.selected.material() {
            Some(material) => {
                // Built parts block the aim, the ground only a little: the
                // lower stones of a foundation lie mostly in the ground and
                // are aimed at through it.
                let blocked = solid.map_or(self.reach, |(_, _, t)| t.min(self.reach));
                let buried = ground.map_or(f32::INFINITY, |t| t + FOUNDATION_DEPTH + 0.5);
                let hits = self.ray_parts_all(eye, dir, blocked.min(buried) + 0.05, 0.04, |p, s| {
                    let (rp, rs) = self.resolve(p, s);
                    !self.pieces[rp].filled[rs]
                        && self.is_shown(rp, rs)
                        && self.blueprints[&self.pieces[rp].kind].slots[rs].material == material
                });
                // Of the outlines along the ray, the first part that can go
                // in now (a wall's part waits for its foundation).
                let part = hits
                    .iter()
                    .find(|(p, s, _)| self.part_problem(*p, *s).is_none())
                    .or(hits.first())
                    .copied();
                self.aim = match part {
                    Some((piece, slot, _)) => {
                        self.target = Some(piece);
                        Aim::Part { piece, slot, problem: self.part_problem(piece, slot) }
                    }
                    None => Aim::Nothing,
                };
            }
            None => {
                let hit = any.map(|(_, _, t)| t).into_iter().chain(ground).fold(f32::INFINITY, f32::min);
                if !hit.is_finite() {
                    self.aim = Aim::Nothing;
                    return;
                }
                let point = eye + dir * hit;
                self.aim = match self.blueprint {
                    PieceKind::Foundation => self.aim_foundation(point),
                    kind => self.aim_wall(kind, point),
                };
            }
        }
    }

    /// Where a foundation goes for the crosshair at `point`: beside a
    /// foundation near it, or on its own on the ground.
    fn aim_foundation(&self, point: Vector3<f32>) -> Aim {
        let kind = PieceKind::Foundation;
        let mut best: Option<(f32, Vector3<f32>, f32)> = None;
        for piece in self.pieces.iter().filter(|p| p.kind == kind) {
            if horizontal(point - piece.position) > FOUNDATION_SIZE * 1.6 {
                continue;
            }
            for k in 0..4 {
                let at = foundation_beside(piece, k);
                let d = horizontal(point - at);
                if best.map_or(true, |(b, _, _)| d < b) {
                    best = Some((d, at, piece.yaw));
                }
            }
        }
        if let Some((d, position, yaw)) = best.filter(|(d, _, _)| *d < FOUNDATION_SIZE * 0.75) {
            let _ = d;
            let taken = self
                .pieces
                .iter()
                .any(|p| p.kind == kind && horizontal(p.position - position) < 0.5);
            return Aim::Place {
                kind,
                position,
                yaw,
                foundation: None,
                problem: taken.then_some("Hier steht schon ein Fundament"),
            };
        }
        // On its own: facing the player, as high as the highest ground
        // under it.
        let yaw = FRAC_PI_2 - self.yaw + self.turn;
        let rotation = Quaternion::from_angle_y(Rad(yaw));
        let half = FOUNDATION_SIZE / 2.0;
        let heights: Vec<f32> = [-1.0f32, 0.0, 1.0]
            .iter()
            .flat_map(|a| [-1.0f32, 0.0, 1.0].map(|b| (*a, b)))
            .map(|(a, b)| {
                let p = point + rotation * Vector3::new(a * half, 0.0, b * half);
                self.terrain_height(p.x, p.z)
            })
            .collect();
        let high = heights.iter().copied().fold(f32::MIN, f32::max);
        let low = heights.iter().copied().fold(f32::MAX, f32::min);
        let position = Vector3::new(point.x, high + 0.55, point.z);
        let problem = if position.y - low > FOUNDATION_DEPTH - 0.1 {
            Some("Der Boden ist hier zu steil")
        } else if self
            .pieces
            .iter()
            .any(|p| p.kind == kind && horizontal(p.position - position) < FOUNDATION_SIZE * 1.2)
        {
            Some("Zu nah an einem anderen Fundament")
        } else {
            None
        };
        Aim::Place { kind, position, yaw, foundation: None, problem }
    }

    /// Where a wall goes for the crosshair at `point`: on the nearest free
    /// edge of a foundation.
    fn aim_wall(&self, kind: PieceKind, point: Vector3<f32>) -> Aim {
        let mut best: Option<(f32, Vector3<f32>, f32, usize, Vector3<f32>)> = None;
        for (f, piece) in self.pieces.iter().enumerate() {
            if piece.kind != PieceKind::Foundation {
                continue;
            }
            for k in 0..4 {
                let (position, yaw) = wall_on_edge(piece, k);
                let edge = position + Quaternion::from_angle_y(Rad(yaw)) * Vector3::new(0.0, 0.0, WALL_THICKNESS / 2.0);
                let d = horizontal(point - edge) + (point.y - edge.y).abs() * 0.5;
                if best.map_or(true, |(b, ..)| d < b) {
                    best = Some((d, position, yaw, f, edge));
                }
            }
        }
        match best.filter(|(d, ..)| *d < 2.4) {
            Some((_, position, yaw, foundation, edge)) => {
                let taken = self
                    .pieces
                    .iter()
                    .any(|p| p.kind.is_wall() && (p.edge_middle() - edge).magnitude() < 0.2);
                Aim::Place {
                    kind,
                    position,
                    yaw,
                    foundation: Some(foundation),
                    problem: taken.then_some("Hier steht schon eine Wand"),
                }
            }
            None => Aim::Place {
                kind,
                position: point,
                yaw: FRAC_PI_2 - self.yaw + PI,
                foundation: None,
                problem: Some("Wände gehören auf die Kante eines Fundaments"),
            },
        }
    }

    fn say(&mut self, text: &str, warning: bool) {
        self.message = Some((text.to_string(), MESSAGE_SECONDS, warning));
    }

    /// The left mouse button, with what is in hand.
    fn use_item(&mut self) {
        match self.aim.clone() {
            Aim::Nothing => {
                if self.selected != Item::Blueprint {
                    self.say("Ziele auf einen Bauplan, um Teile einzusetzen", true);
                }
            }
            Aim::Place { problem: Some(problem), .. } => self.say(problem, true),
            Aim::Place { kind, position, yaw, foundation, problem: None } => {
                self.place(kind, position, yaw, foundation);
                let text = format!("Bauplan gesetzt: {}", kind.name());
                self.say(&text, false);
            }
            Aim::Part { piece, slot, .. } => match self.fill(piece, slot) {
                Ok(()) => {
                    if self.is_complete(piece) {
                        let text = format!("{} fertig gebaut", self.pieces[piece].kind.name());
                        self.say(&text, false);
                    } else if let Some((_, time, _)) = &mut self.message {
                        // Building on: the last message makes room.
                        *time = time.min(0.4);
                    }
                }
                Err(problem) => self.say(problem, true),
            },
        }
    }

    // ── Moving and looking ──

    fn update_camera(&mut self, scene: &mut Scene, delta_time: f32) {
        let Some(camera) = scene.get_component_mut::<CameraComponent>() else {
            return;
        };
        camera.get_camera_controller_mut().is_free = false;
        let eye = match self.eye {
            Some(eye) => eye,
            None => {
                self.yaw = self.start_yaw.to_radians();
                self.pitch = -0.15;
                let [x, z] = self.start;
                let eye = Vector3::new(x, self.floor_height(x, z) + EYE_HEIGHT, z);
                self.eye = Some(eye);
                eye
            }
        };
        if self.radial.is_none() {
            let (dx, dy) = std::mem::take(&mut self.look);
            let turn = self.mouse_sensitivity.to_radians();
            self.yaw += dx * turn;
            self.pitch = (self.pitch - dy * turn).clamp(-1.5, 1.5);
        }
        let (sy, cy) = self.yaw.sin_cos();
        let forward = Vector3::new(cy, 0.0, sy);
        let right = Vector3::new(-sy, 0.0, cy);
        let k = self.keys;
        let axis = |a: bool, b: bool| a as i32 as f32 - b as i32 as f32;
        let mut wish = forward * axis(k.forward, k.back) + right * axis(k.right, k.left);
        if wish.magnitude2() > 1.0 {
            wish = wish.normalize();
        }
        let mut eye = eye;
        if self.flying {
            eye += wish * self.fly_speed * delta_time;
            eye.y += axis(k.up, k.down) * self.fly_speed * delta_time;
            let floor = self.floor_height(eye.x, eye.z) + 0.3;
            eye.y = eye.y.max(floor);
            self.vertical_speed = 0.0;
        } else {
            eye += wish * self.walk_speed * delta_time;
            let floor = self.floor_height(eye.x, eye.z) + EYE_HEIGHT;
            let on_ground = eye.y <= floor + 0.05;
            if on_ground && std::mem::take(&mut self.jump) {
                self.vertical_speed = JUMP_SPEED;
            }
            self.vertical_speed -= GRAVITY * delta_time;
            eye.y += self.vertical_speed * delta_time;
            if eye.y < floor {
                // Steps up onto a foundation or a slope at once.
                eye.y = floor;
                self.vertical_speed = 0.0;
            }
        }
        self.jump = false;
        self.eye = Some(eye);
        let camera = camera.get_camera_mut();
        camera.set_relative_position(Point3::new(0.0, 0.0, 0.0));
        camera.set_position(Point3::from_vec(eye));
        camera.set_yaw_pitch(Rad(self.yaw), Rad(self.pitch));
    }

    /// Make the parts' meshes, once (needs a GPU).
    fn make_models(&mut self) {
        if !self.models.is_empty() || headless::is_headless() {
            return;
        }
        for blueprint in self.blueprints.values() {
            for slot in &blueprint.slots {
                let key = slot.shape();
                if self.models.contains_key(&key) {
                    continue;
                }
                match generated::build_model(&[("part", &key.mesh())]) {
                    Ok(mut model) => {
                        for material in 0..model.material_count().max(1) {
                            model.set_material_color(material, key.color());
                        }
                        self.models.insert(key, RefCell::new(model));
                    }
                    Err(e) => ferrite::log::warn!("builder: {e}"),
                }
            }
        }
    }

    /// Draw part `slot` of `piece`, tinted see-through or as it is.
    fn draw_part(&self, piece: &Piece, slot: usize, tint: Option<[f32; 4]>, lighting: &Lighting, view_projection: &Matrix4<f32>) {
        let slot = &self.blueprints[&piece.kind].slots[slot];
        let Some(model) = self.models.get(&slot.shape()) else {
            return;
        };
        let transform = piece.transform() * Matrix4::from_translation(slot.center);
        if tint.is_some() {
            model.borrow_mut().set_tint(tint);
        }
        model.borrow().render(lighting, &transform, view_projection);
        if tint.is_some() {
            model.borrow_mut().set_tint(None);
        }
    }
}

impl Behaviour for Builder {
    fn update(&mut self, scene: &mut Scene, _entity: &mut Entity, delta_time: f64) {
        self.playing = true;
        self.make_models();
        if self.generator.is_none() {
            self.generator = scene
                .get_component::<Terrain<DualContouringChunk>>()
                .map(|terrain| terrain.generator().clone());
        }
        let dt = (delta_time as f32).min(0.1);
        self.update_camera(scene, dt);
        self.compute_aim();
        for _ in 0..std::mem::take(&mut self.clicks) {
            self.use_item();
            self.compute_aim();
        }
        if let Some((_, time, _)) = &mut self.message {
            *time -= dt;
            if *time <= 0.0 {
                self.message = None;
            }
        }
    }

    fn render(&self, scene: &Scene, _entity: &Entity, view_projection: &Matrix4<f32>, _parent: &Matrix4<f32>) {
        let Some(lighting) = Lighting::from_scene(scene) else {
            return;
        };
        for (p, piece) in self.pieces.iter().enumerate() {
            for s in 0..piece.filled.len() {
                if piece.links[s].is_none() && self.is_filled(p, s) {
                    self.draw_part(piece, s, None, &lighting, view_projection);
                }
            }
        }
    }

    fn render_transparent(&self, scene: &Scene, _entity: &Entity, view_projection: &Matrix4<f32>, _parent: &Matrix4<f32>) {
        let Some(lighting) = Lighting::from_scene(scene) else {
            return;
        };
        let aimed = match &self.aim {
            Aim::Part { piece, slot, problem } => Some((*piece, *slot, problem.is_some())),
            _ => None,
        };
        for (p, piece) in self.pieces.iter().enumerate() {
            for s in 0..piece.filled.len() {
                if piece.links[s].is_some() || self.is_filled(p, s) || !self.is_shown(p, s) {
                    continue;
                }
                let tint = match aimed {
                    Some((ap, as_, blocked)) if ap == p && as_ == s => {
                        if blocked {
                            GHOST_BLOCKED
                        } else {
                            GHOST_AIMED
                        }
                    }
                    _ => GHOST,
                };
                self.draw_part(piece, s, Some(tint), &lighting, view_projection);
            }
        }
        // While the radial menu is open, nothing is about to be placed.
        if let (Aim::Place { kind, position, yaw, problem, .. }, None) = (&self.aim, self.radial) {
            let blueprint = &self.blueprints[kind];
            let preview = Piece::new(*kind, *position, *yaw, blueprint);
            let tint = if problem.is_some() { PREVIEW_BAD } else { PREVIEW };
            for s in 0..blueprint.slots.len() {
                self.draw_part(&preview, s, Some(tint), &lighting, view_projection);
            }
        }
    }

    fn render_overlay(&self, _scene: &Scene, _entity: &Entity) {
        if !self.playing {
            return;
        }
        let target_name;
        let target = match self.target {
            Some(p) => {
                let (done, total) = self.progress(p);
                let kind = self.pieces[p].kind;
                target_name = format!("{} · {}", kind.name(), kind.material().plural());
                Some((target_name.as_str(), done, total))
            }
            None => None,
        };
        let hint = if self.radial.is_some() {
            "Maus: Bauplan wählen  ·  Klick: übernehmen"
        } else {
            match self.selected {
                Item::Blueprint => match self.blueprint {
                    PieceKind::Foundation => "Linksklick: Bauplan setzen  ·  Rechtsklick: Bauplan wählen  ·  R: drehen",
                    _ => "Linksklick: Bauplan setzen  ·  Rechtsklick: Bauplan wählen",
                },
                Item::Stones => "Linksklick: Stein an der markierten Stelle einsetzen",
                Item::Beams => "Linksklick: Balken an der markierten Stelle einsetzen",
            }
        };
        let message = self.message.as_ref().map(|(text, time, warning)| (text.as_str(), time.min(0.5) * 2.0, *warning));
        self.hud.draw(&HudState {
            selected: self.selected,
            blueprint: self.blueprint,
            radial: self.radial,
            target,
            hint,
            message,
        });
    }

    fn handle_event(&mut self, _glfw: &mut glfw::Glfw, window: &mut glfw::Window, event: &WindowEvent) {
        match event {
            WindowEvent::Key(key, _, action, _) => {
                let down = match action {
                    Action::Press => true,
                    Action::Release => false,
                    Action::Repeat => return,
                };
                match key {
                    Key::W => self.keys.forward = down,
                    Key::S => self.keys.back = down,
                    Key::A => self.keys.left = down,
                    Key::D => self.keys.right = down,
                    Key::Space => {
                        self.keys.up = down;
                        self.jump |= down;
                    }
                    Key::LeftShift => self.keys.down = down,
                    _ if !down => {}
                    Key::Num1 => self.select(Item::Blueprint, self.blueprint),
                    Key::Num2 => self.select(Item::Stones, self.blueprint),
                    Key::Num3 => self.select(Item::Beams, self.blueprint),
                    Key::F => self.flying = !self.flying,
                    Key::R => self.turn += 15f32.to_radians(),
                    Key::Escape => self.release_cursor = !self.release_cursor,
                    _ => {}
                }
                if matches!(key, Key::Escape) && down {
                    window.set_cursor_mode(if self.release_cursor {
                        glfw::CursorMode::Normal
                    } else {
                        glfw::CursorMode::Disabled
                    });
                    self.last_cursor = None;
                }
            }
            WindowEvent::Scroll(_, y) if self.radial.is_none() && *y != 0.0 => {
                let i = Item::ALL.iter().position(|i| *i == self.selected).unwrap_or(0) as i32;
                let n = Item::ALL.len() as i32;
                let next = (i - y.signum() as i32).rem_euclid(n);
                self.select(Item::ALL[next as usize], self.blueprint);
            }
            WindowEvent::MouseButton(button, action, _) => {
                let pressed = *action == Action::Press;
                let choice = self
                    .radial
                    .and_then(|cursor| radial_choice(cursor, PieceKind::ALL.len(), RADIAL_DEAD_ZONE));
                match (button, pressed, self.radial) {
                    // Opening the menu with the blueprint in hand.
                    (MouseButton::Button2, true, None) if self.selected == Item::Blueprint => {
                        self.radial = Some((0.0, 0.0));
                    }
                    // Letting go on an option, or clicking, picks it.
                    (MouseButton::Button2, false, Some(_)) | (MouseButton::Button1, true, Some(_)) | (MouseButton::Button2, true, Some(_)) => {
                        if let Some(choice) = choice {
                            self.blueprint = PieceKind::ALL[choice];
                            self.turn = 0.0;
                            self.radial = None;
                        } else if pressed {
                            self.radial = None;
                        }
                    }
                    (MouseButton::Button1, true, None) => self.clicks += 1,
                    _ => {}
                }
            }
            WindowEvent::CursorPos(x, y) => {
                let captured = window.get_cursor_mode() == glfw::CursorMode::Disabled;
                if let (true, Some((lx, ly))) = (captured, self.last_cursor) {
                    let (dx, dy) = ((x - lx) as f32, (y - ly) as f32);
                    match &mut self.radial {
                        Some((cx, cy)) => {
                            let (nx, ny) = (*cx + dx, *cy + dy);
                            let length = (nx * nx + ny * ny).sqrt();
                            let scale = if length > RADIAL_RADIUS { RADIAL_RADIUS / length } else { 1.0 };
                            *cx = nx * scale;
                            *cy = ny * scale;
                        }
                        None => {
                            self.look.0 += dx;
                            self.look.1 += dy;
                        }
                    }
                }
                self.last_cursor = Some((*x, *y));
            }
            _ => {}
        }
    }
}
