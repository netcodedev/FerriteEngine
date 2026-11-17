//! Power: put up producers (solar panels, wind turbines, generators),
//! batteries and consumers (lamps, floodlights, heaters), and join them
//! with cables drawn by hand from port to port. A cable is fastened to
//! walls, floors or the ground at every click on the way, each span sags
//! as much as the mouse wheel says, and a cable is only as long as its
//! reel. Poles carry an overhead line; their boxes branch power off it.
//!
//! Controls:
//!
//! | Input | |
//! | --- | --- |
//! | 1 … 9 | Cable, solar panel, wind turbine, generator, battery, lamp, floodlight, heater, pole |
//! | Left click (cable) | On a port: start a cable. Then: fasten it where the crosshair is, or join it to a port |
//! | Mouse wheel (cable) | How far the next span sags |
//! | Shift (cable) | Hold the next span exactly level or upright |
//! | Right click (cable) | Take back the last point; with none, drop the cable |
//! | Left click (device) | Put the device up; R turns it |
//! | E | Switch the device under the crosshair on or off |
//! | X | Take down the device or cable under the crosshair |
//! | T, G | Time lapse; next weather (clear, rain, snow) |
//! | WASD, Space, mouse | Walk, jump, look; F flies (Space, Ctrl up and down) |
//! | Esc | Free or catch the mouse |

use std::cell::RefCell;
use std::collections::HashMap;
use std::f32::consts::FRAC_PI_2;
use std::sync::Arc;

use ferrite::cgmath::Matrix3;
use ferrite::core::{
    entity::{component::camera_component::CameraComponent, EntityHandle},
    headless,
    model::{generated, Model},
    physics::physics_engine::{PhysicsEngine, RayHit},
    renderer::{
        light::{
            light::{Light, LightKind, LightSettings},
            skylight::SkyLight,
        },
        pbr::Lighting,
        weather::{Precipitation, WeatherMode},
    },
};
use ferrite::glfw;
use ferrite::prelude::*;
use ferrite::rapier3d::math::{Pose, Rotation as PhysicsRotation, Vector as PhysicsVector};
use ferrite::rapier3d::prelude::{ColliderBuilder, ColliderHandle};
use ferrite::terrain::{source::TerrainSource, Terrain};

use crate::cable::{self, horizontal, lock, ray_segment, sag_for, Cable, End, Lock, Span};
use crate::devices::{self, parts, DeviceKind, Glow, Mount, Role};
use crate::grid::{self, Flow, Network, Unit};
use crate::power_hud::{self, CableInfo, Hud, HudState, Item, Panel};
use crate::sources::{self, Sky};

/// See-through looks, `(r, g, b, opacity)`.
const PREVIEW: [f32; 4] = [0.40, 0.78, 1.0, 0.38];
const PREVIEW_BAD: [f32; 4] = [1.0, 0.30, 0.22, 0.42];
const CABLE_PREVIEW: [f32; 4] = [0.45, 0.85, 1.0, 0.85];
const CABLE_PREVIEW_BAD: [f32; 4] = [1.0, 0.32, 0.22, 0.85];
const PORT: [f32; 4] = [1.0, 0.80, 0.25, 0.75];
const PORT_AIMED: [f32; 4] = [2.2, 1.8, 0.6, 1.0];
const PORT_BUSY: [f32; 4] = [0.45, 0.95, 0.45, 0.75];

const EYE_HEIGHT: f32 = 1.7;
const GRAVITY: f32 = 20.0;
const JUMP_SPEED: f32 = 6.5;
const MESSAGE_SECONDS: f32 = 2.6;
const CABLE_RADIUS: f32 = 0.018;
const OVERHEAD_RADIUS: f32 = 0.014;
/// A port counts as aimed at within this distance of the ray.
const PORT_RADIUS: f32 = 0.16;
/// How fast time runs with the time lapse on.
const WARP: f32 = 30.0;
/// How often solar panels look for shadows, seconds.
const SHADOW_INTERVAL: f32 = 0.25;
/// Below this height of the sun the street lamps come on.
const DUSK: f32 = 0.08;
/// Colliders of this demo (see `collider_tag`).
const TAG: u128 = ferrite::core::physics::physics_engine::collider_tag::GAME_TAGS + 70;

/// A device in the world.
pub struct Device {
    pub kind: DeviceKind,
    pub position: Vector3<f32>,
    pub rotation: Quaternion<f32>,
    /// Switched on (`E`).
    pub on: bool,
    /// A battery's charge, watt hours.
    pub charge_wh: f32,
    /// This frame: what a producer could make, what a consumer wants.
    pub available_w: f32,
    pub demand_w: f32,
    /// This frame: what it made, got or stored.
    pub flow: Flow,
    pub network: usize,
    /// A solar panel: share of its face in the sun, 0..1.
    pub lit: f32,
    /// A wind turbine: the rotor's angle, radians.
    pub rotor: f32,
    colliders: Vec<ColliderHandle>,
    light: Option<EntityHandle>,
}

impl Device {
    fn new(kind: DeviceKind, position: Vector3<f32>, rotation: Quaternion<f32>) -> Device {
        Device {
            kind,
            position,
            rotation,
            on: true,
            charge_wh: kind.capacity_wh() * 0.5,
            available_w: 0.0,
            demand_w: 0.0,
            flow: Flow::default(),
            network: 0,
            lit: 1.0,
            rotor: 0.0,
            colliders: Vec::new(),
            light: None,
        }
    }

    pub fn transform(&self) -> Matrix4<f32> {
        Matrix4::from_translation(self.position) * Matrix4::from(self.rotation)
    }

    pub fn to_world(&self, local: Vector3<f32>) -> Vector3<f32> {
        self.position + self.rotation * local
    }

    pub fn port(&self, i: usize) -> Vector3<f32> {
        self.to_world(Vector3::from(self.kind.ports()[i]))
    }

    /// How brightly its lights and glowing parts shine, 0..1.
    pub fn glow(&self) -> f32 {
        match self.kind.role() {
            Role::Consumer | Role::Line => self.flow.supplied,
            Role::Producer => (self.kind == DeviceKind::Generator && self.on) as i32 as f32,
            Role::Storage => 1.0,
        }
    }
}

/// What a block of a building is made of (its colour).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BlockLook {
    Floor,
    Wall,
    Roof,
}

impl BlockLook {
    const ALL: [BlockLook; 3] = [BlockLook::Floor, BlockLook::Wall, BlockLook::Roof];

    fn color(self) -> [f32; 3] {
        match self {
            BlockLook::Floor => [0.50, 0.36, 0.23],
            BlockLook::Wall => [0.80, 0.75, 0.66],
            BlockLook::Roof => [0.28, 0.29, 0.31],
        }
    }
}

/// A box of a building (the shed): something to fasten cables to.
pub struct Block {
    pub centre: Vector3<f32>,
    pub half: Vector3<f32>,
    pub yaw: f32,
    pub look: BlockLook,
    collider: Option<ColliderHandle>,
}

impl Block {
    fn rotation(&self) -> Quaternion<f32> {
        Quaternion::from_angle_y(Rad(self.yaw))
    }

    fn transform(&self) -> Matrix4<f32> {
        Matrix4::from_translation(self.centre) * Matrix4::from(self.rotation()) * Matrix4::from_nonuniform_scale(2.0 * self.half.x, 2.0 * self.half.y, 2.0 * self.half.z)
    }
}

/// Where the next point of the cable being drawn goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NextPoint {
    pub point: Vector3<f32>,
    /// A port: the cable ends there.
    pub end: Option<End>,
    pub lock: Option<Lock>,
    pub problem: Option<&'static str>,
}

/// What a left click would do now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Aim {
    Nothing,
    /// Start a cable at this port.
    Port(End),
    /// The next point of the cable being drawn.
    Next(NextPoint),
    /// Put a device up here.
    Place {
        kind: DeviceKind,
        position: Vector3<f32>,
        rotation: Quaternion<f32>,
        problem: Option<&'static str>,
    },
}

/// What the crosshair is on, for `E`, `X` and the HUD.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    Device(usize, Option<usize>),
    Cable(usize),
}

/// The cable being drawn: where it starts, the points fastened so far
/// (its port first) and their spans' sags.
#[derive(Clone, Debug)]
pub struct Drawing {
    pub from: End,
    pub points: Vec<Vector3<f32>>,
    pub sags: Vec<f32>,
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

/// Inputs gathered between updates.
#[derive(Default, Clone, Copy)]
struct Pending {
    clicks: usize,
    right_clicks: usize,
    scroll: f32,
    toggle: bool,
    remove: bool,
    weather: bool,
}

/// The player's power work: devices, cables and the grid between them.
#[derive(Script)]
pub struct Electrician {
    /// Walking speed, metres per second.
    pub walk_speed: f32,
    /// Flying speed (F), metres per second.
    pub fly_speed: f32,
    /// How far away things can be put up, cables fastened, ports reached.
    pub reach: f32,
    /// Degrees the view turns per pixel of mouse movement.
    pub mouse_sensitivity: f32,
    /// Where the player starts: x and z (stands on the ground there).
    pub start: [f32; 2],
    /// Which way the player looks at the start, degrees.
    pub start_yaw: f32,
    /// How long one cable can be, metres.
    pub cable_length: f32,
    /// The poles standing along the (future) road: first pole x and z,
    /// direction in degrees, count, spacing in metres. They are joined by
    /// an overhead line, fed by a wind turbine and a battery at the first.
    pub pole_line: [f32; 5],
    /// Put up the demo's shed, poles and turbine at the start.
    pub setup: bool,

    devices: Vec<Device>,
    cables: Vec<Cable>,
    blocks: Vec<Block>,
    networks: Vec<Network>,
    /// The colliders of devices and blocks: which device, if any.
    collider_owner: HashMap<ColliderHandle, usize>,
    /// Colliders and lights of devices taken down, to be released.
    dropped_colliders: Vec<ColliderHandle>,
    dropped_lights: Vec<EntityHandle>,
    source: Option<Arc<dyn TerrainSource>>,
    set_up: bool,

    device_models: HashMap<DeviceKind, Vec<(RefCell<Model>, Glow, bool)>>,
    block_models: HashMap<BlockLook, RefCell<Model>>,
    cable_models: Vec<Option<RefCell<Model>>>,
    piece_model: Option<RefCell<Model>>,
    ball_model: Option<RefCell<Model>>,
    models_made: bool,

    selected: Item,
    drawing: Option<Drawing>,
    /// How far a span sags, as a share of its horizontal length.
    sag_share: f32,
    shift: bool,
    /// Extra turn of devices put up (R).
    turn: f32,
    aim: Aim,
    target: Option<Target>,
    message: Option<(String, f32, bool)>,
    pending: Pending,

    sky: Sky,
    wind: f32,
    clock: f32,
    shadow_timer: f32,
    warp: bool,
    hours_per_second: f32,
    time_of_day: (u32, u32),

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

impl Default for Electrician {
    fn default() -> Self {
        Self {
            walk_speed: 5.0,
            fly_speed: 14.0,
            reach: 9.0,
            mouse_sensitivity: 0.12,
            start: [-12.0, -160.0],
            start_yaw: 0.0,
            cable_length: cable::DEFAULT_MAX_LENGTH,
            pole_line: [-22.0, -151.0, 0.0, 4.0, 22.0],
            setup: true,
            devices: Vec::new(),
            cables: Vec::new(),
            blocks: Vec::new(),
            networks: Vec::new(),
            collider_owner: HashMap::new(),
            dropped_colliders: Vec::new(),
            dropped_lights: Vec::new(),
            source: None,
            set_up: false,
            device_models: HashMap::new(),
            block_models: HashMap::new(),
            cable_models: Vec::new(),
            piece_model: None,
            ball_model: None,
            models_made: false,
            selected: Item::Cable,
            drawing: None,
            sag_share: cable::DEFAULT_SAG_SHARE,
            shift: false,
            turn: 0.0,
            aim: Aim::Nothing,
            target: None,
            message: None,
            pending: Pending::default(),
            sky: Sky::default(),
            wind: 0.0,
            clock: 0.0,
            shadow_timer: 0.0,
            warp: false,
            hours_per_second: 0.02,
            time_of_day: (12, 0),
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

/// The rotation whose +Y is `up` and whose +Z leans toward `forward`.
fn frame(up: Vector3<f32>, forward: Vector3<f32>) -> Quaternion<f32> {
    let y = up.normalize();
    let mut z = forward - y * forward.dot(y);
    if z.magnitude2() < 1e-6 {
        z = if y.x.abs() < 0.9 { Vector3::unit_x().cross(y) } else { Vector3::unit_z().cross(y) };
    }
    let z = z.normalize();
    let x = y.cross(z);
    Quaternion::from(Matrix3::from_cols(x, y, z)).normalize()
}

/// Upright, its front toward `toward` (horizontal), turned by `turn`.
fn upright(toward: Vector3<f32>, turn: f32) -> Quaternion<f32> {
    let yaw = toward.x.atan2(toward.z) + turn;
    Quaternion::from_angle_y(Rad(yaw))
}

fn physics_pose(centre: Vector3<f32>, rotation: Quaternion<f32>) -> Pose {
    Pose::from_parts(
        PhysicsVector::new(centre.x, centre.y, centre.z),
        PhysicsRotation::from_xyzw(rotation.v.x, rotation.v.y, rotation.v.z, rotation.s),
    )
}

fn cast(physics: &mut PhysicsEngine, origin: Vector3<f32>, dir: Vector3<f32>, max: f32, ignore: &[ColliderHandle]) -> Option<RayHit> {
    physics.cast_ray_where(origin.into(), dir.into(), max, |handle, _| !ignore.contains(&handle))
}

/// A piece of cable from `a` to `b`: a unit cylinder along +Z, turned and
/// stretched.
fn piece_transform(a: Vector3<f32>, b: Vector3<f32>, radius: f32) -> Option<Matrix4<f32>> {
    let d = b - a;
    let length = d.magnitude();
    if length < 1e-4 {
        return None;
    }
    let rotation = Quaternion::from_arc(Vector3::unit_z(), d / length, Some(Vector3::unit_x()));
    Some(Matrix4::from_translation(a) * Matrix4::from(rotation) * Matrix4::from_nonuniform_scale(radius, radius, length))
}

fn kw(watts: f32) -> String {
    if watts.abs() >= 1000.0 {
        format!("{:.2} kW", watts / 1000.0)
    } else {
        format!("{:.0} W", watts)
    }
}

impl Electrician {
    // ── For other scripts (and tests): the work without the mouse ──

    pub fn devices(&self) -> &[Device] {
        &self.devices
    }

    pub fn device_mut(&mut self, i: usize) -> &mut Device {
        &mut self.devices[i]
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    pub fn cables(&self) -> &[Cable] {
        &self.cables
    }

    pub fn network(&self, device: usize) -> Option<&Network> {
        self.networks.get(self.devices.get(device)?.network)
    }

    pub fn aim(&self) -> &Aim {
        &self.aim
    }

    pub fn drawing(&self) -> Option<&Drawing> {
        self.drawing.as_ref()
    }

    pub fn select(&mut self, item: Item) {
        if item != Item::Cable && self.drawing.take().is_some() {
            self.say("Kabel verworfen", true);
        }
        self.selected = item;
    }

    /// Stand at `eye` looking along yaw and pitch (radians).
    pub fn set_view(&mut self, eye: Vector3<f32>, yaw: f32, pitch: f32) {
        self.eye = Some(eye);
        self.yaw = yaw;
        self.pitch = pitch;
        self.flying = true;
    }

    /// Look from `eye` at `at`.
    pub fn look_at(&mut self, eye: Vector3<f32>, at: Vector3<f32>) {
        let d = (at - eye).normalize();
        self.set_view(eye, d.z.atan2(d.x), d.y.asin());
    }

    pub fn set_shift(&mut self, down: bool) {
        self.shift = down;
    }

    pub fn set_turn(&mut self, turn: f32) {
        self.turn = turn;
    }

    pub fn set_sag_share(&mut self, share: f32) {
        self.sag_share = share.clamp(0.0, cable::MAX_SAG_SHARE);
    }

    pub fn set_warp(&mut self, warp: bool) {
        self.warp = warp;
    }

    /// Click, as the left mouse button does.
    pub fn click(&mut self) {
        self.pending.clicks += 1;
    }

    pub fn right_click(&mut self) {
        self.pending.right_clicks += 1;
    }

    /// Turn the mouse wheel by `notches` (up is positive).
    pub fn scroll(&mut self, notches: f32) {
        self.pending.scroll += notches;
    }

    /// Put up a device. Returns its index.
    pub fn place(&mut self, kind: DeviceKind, position: Vector3<f32>, rotation: Quaternion<f32>) -> usize {
        self.devices.push(Device::new(kind, position, rotation));
        self.devices.len() - 1
    }

    /// Join two devices' ports with a cable through `between`, every span
    /// sagging by `share` of its length.
    pub fn connect(&mut self, from: End, to: End, between: &[Vector3<f32>], share: f32, overhead: bool) -> usize {
        let mut points = vec![self.devices[from.device].port(from.port)];
        points.extend_from_slice(between);
        points.push(self.devices[to.device].port(to.port));
        let sags = points.windows(2).map(|w| sag_for(w[0], w[1], share)).collect();
        self.cables.push(Cable { from, to, points, sags, overhead });
        self.cable_models.push(None);
        self.cables.len() - 1
    }

    /// Take a device down, with its cables.
    pub fn remove_device(&mut self, index: usize) {
        if index >= self.devices.len() {
            return;
        }
        let device = self.devices.remove(index);
        self.dropped_colliders.extend(device.colliders);
        self.dropped_lights.extend(device.light);
        let mut i = 0;
        while i < self.cables.len() {
            if self.cables[i].joins(index) {
                self.remove_cable(i);
            } else {
                i += 1;
            }
        }
        for c in &mut self.cables {
            for end in [&mut c.from, &mut c.to] {
                if end.device > index {
                    end.device -= 1;
                }
            }
        }
        if let Some(drawing) = &self.drawing {
            if drawing.from.device == index {
                self.drawing = None;
            } else if drawing.from.device > index {
                self.drawing.as_mut().unwrap().from.device -= 1;
            }
        }
        self.collider_owner = self
            .devices
            .iter()
            .enumerate()
            .flat_map(|(d, device)| device.colliders.iter().map(move |h| (*h, d)))
            .collect();
    }

    pub fn remove_cable(&mut self, index: usize) {
        if index < self.cables.len() {
            self.cables.remove(index);
            self.cable_models.remove(index);
        }
    }

    // ── The world ──

    fn ground_height(&self, x: f32, z: f32) -> f32 {
        self.source.as_ref().map_or(0.0, |s| s.surface_height(x as f64, z as f64))
    }

    /// The demo's things at the start: a shed to fasten cables to, a line
    /// of poles fed by a wind turbine with a battery.
    fn set_up_world(&mut self) {
        self.set_up = true;
        if !self.setup {
            return;
        }
        let [sx, sz] = self.start;
        self.build_shed(Vector3::new(sx + 12.0, 0.0, sz - 7.0), 0.0);

        let [px, pz, direction, count, spacing] = self.pole_line;
        let along = Vector3::new(direction.to_radians().cos(), 0.0, direction.to_radians().sin());
        let rotation = Quaternion::from_angle_y(Rad(-direction.to_radians()));
        let mut poles = Vec::new();
        for i in 0..count.max(0.0) as usize {
            let p = Vector3::new(px, 0.0, pz) + along * (i as f32 * spacing);
            let at = Vector3::new(p.x, self.ground_height(p.x, p.z) - 0.3, p.z);
            poles.push(self.place(DeviceKind::Pole, at, rotation));
        }
        for pair in poles.windows(2) {
            for port in [1, 2] {
                self.connect(End { device: pair[0], port }, End { device: pair[1], port }, &[], 0.025, true);
            }
        }
        if let Some(&first) = poles.first() {
            // The turbine and the battery behind the first pole, on the
            // side away from the road (the street lamps hang over it).
            let side = Vector3::new(-along.z, 0.0, along.x);
            let base = self.devices[first].position;
            let turbine_at = base - along * 6.0 - side * 5.0;
            let turbine_at = Vector3::new(turbine_at.x, self.ground_height(turbine_at.x, turbine_at.z), turbine_at.z);
            let turbine = self.place(DeviceKind::Wind, turbine_at, upright(side, 0.0));
            let battery_at = base - along * 2.5 - side * 2.0;
            let battery_at = Vector3::new(battery_at.x, self.ground_height(battery_at.x, battery_at.z), battery_at.z);
            let battery = self.place(DeviceKind::Battery, battery_at, upright(along, 0.0));
            let pole_box = End { device: first, port: 0 };
            self.connect(End { device: turbine, port: 0 }, pole_box, &[], 0.08, false);
            self.connect(End { device: battery, port: 0 }, pole_box, &[], 0.05, false);
        }
    }

    /// A small shed: a floor, walls with a door toward −x, a flat roof.
    fn build_shed(&mut self, centre: Vector3<f32>, yaw: f32) {
        let (hx, hz) = (2.2f32, 1.6f32);
        let rotation = Quaternion::from_angle_y(Rad(yaw));
        let corners = [(-hx, -hz), (hx, -hz), (hx, hz), (-hx, hz), (0.0, 0.0)];
        let heights: Vec<f32> = corners
            .iter()
            .map(|(a, b)| {
                let p = centre + rotation * Vector3::new(*a, 0.0, *b);
                self.ground_height(p.x, p.z)
            })
            .collect();
        let top = heights.iter().copied().fold(f32::MIN, f32::max) + 0.15;
        let bottom = heights.iter().copied().fold(f32::MAX, f32::min) - 0.4;
        let mut add = |local: Vector3<f32>, half: Vector3<f32>, look: BlockLook| {
            self.blocks.push(Block { centre: Vector3::new(centre.x, 0.0, centre.z) + rotation * local, half, yaw, look, collider: None });
        };
        let floor_half_y = (top - bottom) / 2.0;
        add(Vector3::new(0.0, bottom + floor_half_y, 0.0), Vector3::new(hx, floor_half_y, hz), BlockLook::Floor);
        let (height, t) = (2.5f32, 0.07f32);
        let y = top + height / 2.0;
        add(Vector3::new(hx - t, y, 0.0), Vector3::new(t, height / 2.0, hz), BlockLook::Wall);
        add(Vector3::new(0.0, y, hz - t), Vector3::new(hx - 2.0 * t, height / 2.0, t), BlockLook::Wall);
        add(Vector3::new(0.0, y, -hz + t), Vector3::new(hx - 2.0 * t, height / 2.0, t), BlockLook::Wall);
        // The front, with a door 1 m wide and 2.1 m high.
        let door = 0.5f32;
        let side = (hz - door) / 2.0;
        add(Vector3::new(-hx + t, y, -door - side), Vector3::new(t, height / 2.0, side), BlockLook::Wall);
        add(Vector3::new(-hx + t, y, door + side), Vector3::new(t, height / 2.0, side), BlockLook::Wall);
        let lintel = height - 2.1;
        add(Vector3::new(-hx + t, top + 2.1 + lintel / 2.0, 0.0), Vector3::new(t, lintel / 2.0, door), BlockLook::Wall);
        add(Vector3::new(0.0, top + height + 0.06, 0.0), Vector3::new(hx + 0.25, 0.06, hz + 0.25), BlockLook::Roof);
    }

    /// Give new devices and blocks their colliders, release those of
    /// devices taken down.
    fn sync_colliders(&mut self, physics: &mut PhysicsEngine) {
        for handle in self.dropped_colliders.drain(..) {
            physics.remove_collider(handle);
        }
        for block in &mut self.blocks {
            if block.collider.is_none() {
                let collider = ColliderBuilder::cuboid(block.half.x, block.half.y, block.half.z)
                    .position(physics_pose(block.centre, block.rotation()))
                    .user_data(TAG)
                    .build();
                block.collider = Some(physics.add_collider(collider, None));
            }
        }
        for (d, device) in self.devices.iter_mut().enumerate() {
            if !device.colliders.is_empty() {
                continue;
            }
            for (centre, half, rotation) in device.kind.boxes() {
                let collider = ColliderBuilder::cuboid(half.x, half.y, half.z)
                    .position(physics_pose(device.to_world(centre), device.rotation * rotation))
                    .user_data(TAG)
                    .build();
                let handle = physics.add_collider(collider, None);
                device.colliders.push(handle);
                self.collider_owner.insert(handle, d);
            }
        }
    }

    /// Give lamps, floodlights, heaters and street lamps their lights, and
    /// set how bright they are.
    fn sync_lights(&mut self, entity: &mut Entity) {
        for id in self.dropped_lights.drain(..) {
            entity.remove_descendant(&id);
        }
        for device in &mut self.devices {
            let Some((anchor, dir)) = device.kind.light_anchor() else {
                continue;
            };
            let glow = device.glow();
            let mut settings = match device.kind {
                DeviceKind::Lamp => LightSettings { kind: LightKind::Point, color: [1.0, 0.84, 0.62], intensity: 5.0, range: 9.0, ..LightSettings::default() },
                DeviceKind::Floodlight => LightSettings {
                    kind: LightKind::Spot,
                    color: [0.95, 0.97, 1.0],
                    intensity: 40.0,
                    range: 32.0,
                    inner_angle: 16.0,
                    outer_angle: 32.0,
                    ..LightSettings::default()
                },
                DeviceKind::Heater => LightSettings { kind: LightKind::Point, color: [1.0, 0.38, 0.12], intensity: 0.8, range: 2.5, shadows: false, ..LightSettings::default() },
                _ => LightSettings {
                    kind: LightKind::Spot,
                    color: [1.0, 0.80, 0.55],
                    intensity: 26.0,
                    range: 20.0,
                    inner_angle: 38.0,
                    outer_angle: 62.0,
                    shadows: false,
                    ..LightSettings::default()
                },
            };
            settings.intensity *= glow;
            settings.enabled = glow > 0.01;
            match device.light {
                Some(id) => {
                    if let Some(light) = entity.get_child_mut(&id).and_then(|child| child.get_component_mut::<Light>()) {
                        if light.settings().intensity != settings.intensity || light.settings().enabled != settings.enabled {
                            light.set_settings(settings);
                        }
                    }
                }
                None => {
                    let mut child = Entity::new(&format!("{} light", device.kind.name()));
                    child.set_transient(true);
                    child.set_local_position(Point3::from_vec(device.to_world(anchor)));
                    let down = device.rotation * dir;
                    child.set_local_rotation(Quaternion::from_arc(-Vector3::unit_y(), down, Some(Vector3::unit_x())));
                    child.add_component(Light::new(settings));
                    device.light = Some(child.id);
                    entity.add_child(child);
                }
            }
        }
    }

    // ── The sky and the grid ──

    fn read_sky(&mut self, scene: &mut Scene, dt: f32) {
        let Some(skylight) = scene.get_component_mut::<SkyLight>() else {
            return;
        };
        if std::mem::take(&mut self.pending.weather) {
            let next = match skylight.weather().mode {
                WeatherMode::Clear => WeatherMode::Rain,
                WeatherMode::Rain => WeatherMode::Snow,
                _ => WeatherMode::Clear,
            };
            let mut weather = *skylight.weather();
            weather.mode = next;
            weather.settle();
            skylight.set_weather(weather);
            let name = match next {
                WeatherMode::Clear => "Wetter: klar",
                WeatherMode::Rain => "Wetter: Regen",
                _ => "Wetter: Schnee",
            };
            self.say(name, false);
        }
        let day = skylight.get_day_length_seconds().max(1.0);
        if self.warp {
            skylight.advance_time(dt * (WARP - 1.0) / day);
        }
        self.hours_per_second = 24.0 / day * if self.warp { WARP } else { 1.0 };
        self.time_of_day = skylight.get_clock();
        let (kind, amount) = skylight.weather().precipitation();
        self.sky = Sky {
            sun: skylight.get_sun_direction(),
            clouds: skylight.effective_cloud_coverage(),
            rain: if kind == Precipitation::Rain { amount } else { 0.0 },
            snow: if kind == Precipitation::Snow { amount } else { 0.0 },
            snow_cover: skylight.weather().snow_cover,
        };
    }

    /// Which share of each solar panel's face the sun reaches: rays from a
    /// grid of points on it toward the sun, past anything in the world
    /// (terrain, buildings, other devices, trees).
    fn find_shadows(&mut self, physics: &mut PhysicsEngine) {
        let sun = self.sky.sun;
        for device in self.devices.iter_mut().filter(|d| d.kind == DeviceKind::Solar) {
            if sun.y <= 0.0 {
                device.lit = 0.0;
                continue;
            }
            let tilt = devices::solar_tilt();
            let normal = device.rotation * devices::solar_normal();
            let [hw, hd] = devices::SOLAR_HALF;
            let mut lit = 0;
            let mut total = 0;
            for a in [-0.8f32, 0.0, 0.8] {
                for b in [-0.8f32, 0.0, 0.8] {
                    let local = devices::SOLAR_CENTRE + tilt * Vector3::new(a * hw, 0.04, b * hd);
                    let origin = device.to_world(local) + normal * 0.02;
                    total += 1;
                    if cast(physics, origin, sun, 400.0, &device.colliders).is_none() {
                        lit += 1;
                    }
                }
            }
            device.lit = lit as f32 / total as f32;
        }
    }

    /// Every producer's and consumer's watts for this frame, then the
    /// grid's balance.
    fn run_grid(&mut self, dt: f32) {
        self.wind = sources::wind_speed(self.clock, &self.sky);
        let night = self.sky.sun.y < DUSK;
        let mut units = Vec::with_capacity(self.devices.len());
        for device in &mut self.devices {
            let kind = device.kind;
            let unit = match kind {
                DeviceKind::Solar => {
                    let normal = device.rotation * devices::solar_normal();
                    Unit::producer(sources::solar_watts(kind.watts(), normal, device.lit, &self.sky))
                }
                DeviceKind::Wind => Unit::producer(sources::wind_watts(kind.watts(), self.wind)),
                DeviceKind::Generator => Unit::producer(if device.on { kind.watts() } else { 0.0 }),
                DeviceKind::Battery => Unit::battery(device.charge_wh, kind.capacity_wh(), kind.watts()),
                DeviceKind::Pole => Unit { demand_w: if night { kind.watts() } else { 0.0 }, ..Unit::idle(Role::Line) },
                _ => Unit::consumer(if device.on { kind.watts() } else { 0.0 }),
            };
            device.available_w = unit.available_w;
            device.demand_w = unit.demand_w;
            units.push(unit);
            // The rotor turns with the wind, and stands still in a storm.
            if kind == DeviceKind::Wind {
                let speed = if self.wind > sources::CUT_OUT { 0.2 } else { self.wind.min(sources::RATED) * 0.85 };
                device.rotor = (device.rotor + speed * dt) % std::f32::consts::TAU;
            }
        }
        let links: Vec<(usize, usize)> = self.cables.iter().map(|c| (c.from.device, c.to.device)).collect();
        let nets = grid::networks(units.len(), &links);
        let (flows, networks) = grid::balance(&mut units, &nets, dt * self.hours_per_second);
        for ((device, flow), (unit, net)) in self.devices.iter_mut().zip(flows).zip(units.iter().zip(&nets)) {
            device.flow = flow;
            device.network = *net;
            device.charge_wh = unit.charge_wh;
        }
        self.networks = networks;
    }

    // ── Aiming ──

    /// What the crosshair is on and what a click would do.
    fn compute_aim(&mut self, physics: &mut PhysicsEngine) {
        self.target = None;
        let Some(eye) = self.eye else {
            self.aim = Aim::Nothing;
            return;
        };
        let dir = view_direction(self.yaw, self.pitch);
        let reach = self.reach;
        let surface = cast(physics, eye, dir, reach, &[]);
        let surface_t = surface.map_or(reach, |h| h.distance);
        // The port nearest to the ray, not behind a wall.
        let mut port: Option<(End, f32, f32)> = None;
        for (d, device) in self.devices.iter().enumerate() {
            if (device.position - eye).magnitude() > reach + 9.0 {
                continue;
            }
            for p in 0..device.kind.ports().len() {
                let at = device.port(p);
                let along = (at - eye).dot(dir);
                if along < 0.0 || along > reach || along > surface_t + 0.3 {
                    continue;
                }
                let off = (eye + dir * along - at).magnitude();
                let radius = PORT_RADIUS * (1.0 + along / 12.0);
                if off < radius && port.map_or(true, |(_, _, o)| off < o) {
                    port = Some((End { device: d, port: p }, along, off));
                }
            }
        }
        // A cable under the crosshair.
        let mut cable_hit: Option<(usize, f32)> = None;
        for (c, cable) in self.cables.iter().enumerate() {
            for span in cable.spans() {
                let points = span.points(span.pieces());
                for w in points.windows(2) {
                    let (off, t) = ray_segment(eye, dir, w[0], w[1]);
                    let tolerance = 0.06 + t * 0.008;
                    if off < tolerance && t < surface_t + 0.05 && t < reach && cable_hit.map_or(true, |(_, best)| t < best) {
                        cable_hit = Some((c, t));
                    }
                }
            }
        }
        let device_hit = surface.and_then(|h| self.collider_owner.get(&h.collider).copied());
        self.target = match (port, device_hit, cable_hit) {
            (Some((end, _, _)), _, _) => Some(Target::Device(end.device, Some(end.port))),
            (None, _, Some((c, t))) if t < surface_t - 0.02 || device_hit.is_none() => Some(Target::Cable(c)),
            (None, Some(d), _) => Some(Target::Device(d, None)),
            _ => None,
        };

        self.aim = match (self.selected, &self.drawing) {
            (Item::Cable, Some(drawing)) => {
                let drawing = drawing.clone();
                Aim::Next(self.next_point(physics, &drawing, eye, dir, surface, port.map(|p| p.0)))
            }
            (Item::Cable, None) => match port {
                Some((end, _, _)) => Aim::Port(end),
                None => Aim::Nothing,
            },
            (Item::Device(kind), _) => match surface {
                Some(hit) => self.placement(kind, eye, dir, hit, device_hit),
                None => Aim::Nothing,
            },
        };
    }

    /// Where the next point of `drawing` goes for the crosshair.
    fn next_point(
        &self,
        physics: &mut PhysicsEngine,
        drawing: &Drawing,
        eye: Vector3<f32>,
        dir: Vector3<f32>,
        surface: Option<RayHit>,
        port: Option<End>,
    ) -> NextPoint {
        let last = *drawing.points.last().unwrap();
        let mut next = match (port, surface) {
            (Some(end), _) => NextPoint { point: self.devices[end.device].port(end.port), end: Some(end), lock: None, problem: None },
            (None, Some(hit)) => {
                let normal = Vector3::from(hit.normal);
                let point = eye + dir * hit.distance + normal * 0.03;
                if self.shift {
                    let (locked, lock_kind) = lock(last, point);
                    // The locked point has to lie on something too: what
                    // the eye sees there.
                    let to = locked - eye;
                    let distance = to.magnitude();
                    let held = distance > 0.01
                        && cast(physics, eye, to / distance, distance + 0.5, &[])
                            .is_some_and(|h| (h.distance - distance).abs() < 0.12);
                    NextPoint {
                        point: locked,
                        end: None,
                        lock: Some(lock_kind),
                        problem: (!held).then_some("Hier hält das Kabel nicht"),
                    }
                } else {
                    NextPoint { point, end: None, lock: None, problem: None }
                }
            }
            (None, None) => NextPoint {
                point: eye + dir * self.reach.min(6.0),
                end: None,
                lock: None,
                problem: Some("Ziele auf eine Wand, einen Boden oder einen Anschluss"),
            },
        };
        if next.problem.is_some() {
            return next;
        }
        if let Some(end) = next.end {
            if end.device == drawing.from.device {
                next.problem = Some("Ein Gerät lässt sich nicht mit sich selbst verbinden");
                return next;
            }
        }
        if (next.point - last).magnitude() < 0.15 {
            next.problem = Some("Zu nah am letzten Punkt");
            return next;
        }
        if (next.point - eye).magnitude() > self.reach + 0.5 {
            next.problem = Some("Zu weit weg");
            return next;
        }
        let span = Span::with_share(last, next.point, self.sag_share);
        let length = self.drawn_length(drawing) + span.length();
        if length > self.cable_length {
            next.problem = Some("Das Kabel ist zu lang");
            return next;
        }
        // The span must not run through anything (it may touch where it
        // is fastened).
        let mut ignore: Vec<ColliderHandle> = self.devices[drawing.from.device].colliders.clone();
        if let Some(end) = next.end {
            ignore.extend(&self.devices[end.device].colliders);
        }
        if self.span_blocked(physics, &span, &ignore) {
            next.problem = Some("Das Kabel stößt an ein Hindernis");
        }
        next
    }

    /// Whether `span` runs through something, leaving out a little at
    /// either end.
    fn span_blocked(&self, physics: &mut PhysicsEngine, span: &Span, ignore: &[ColliderHandle]) -> bool {
        let points = span.points(span.pieces().max(8));
        let total: f32 = points.windows(2).map(|w| (w[1] - w[0]).magnitude()).sum();
        let margin = 0.3f32.min(total * 0.3);
        let mut walked = 0.0;
        for w in points.windows(2) {
            let d = w[1] - w[0];
            let length = d.magnitude();
            let (start, end) = (walked, walked + length);
            walked = end;
            if length < 1e-4 {
                continue;
            }
            // The part of this piece away from the ends.
            let from = (margin - start).max(0.0);
            let to = length - (end - (total - margin)).max(0.0);
            if to - from < 1e-3 {
                continue;
            }
            let dir = d / length;
            if cast(physics, w[0] + dir * from, dir, to - from, ignore).is_some() {
                return true;
            }
        }
        false
    }

    fn drawn_length(&self, drawing: &Drawing) -> f32 {
        drawing.points.windows(2).zip(&drawing.sags).map(|(w, sag)| Span { a: w[0], b: w[1], sag: *sag }.length()).sum()
    }

    /// Where a device would go for the crosshair on `hit`.
    fn placement(&self, kind: DeviceKind, eye: Vector3<f32>, dir: Vector3<f32>, hit: RayHit, on_device: Option<usize>) -> Aim {
        let point = eye + dir * hit.distance;
        let normal = Vector3::from(hit.normal);
        let toward = Vector3::new(-dir.x, 0.0, -dir.z);
        let toward = if toward.magnitude2() > 1e-6 { toward.normalize() } else { Vector3::unit_z() };
        let (position, rotation) = match kind.mount() {
            Mount::Floor => (point, upright(toward, self.turn)),
            Mount::Any => (point, frame(normal, toward) * Quaternion::from_angle_y(Rad(self.turn))),
        };
        let problem = if on_device.is_some() {
            Some("Nicht auf ein anderes Gerät")
        } else if kind.mount() == Mount::Floor && normal.y < 0.75 {
            Some("Das braucht ebenen Boden")
        } else {
            self.devices
                .iter()
                .find(|d| {
                    let apart = d.position - position;
                    horizontal(apart) < kind.footprint() + d.kind.footprint() && apart.y.abs() < 2.0
                })
                .map(|_| "Zu nah an einem anderen Gerät")
        };
        Aim::Place { kind, position, rotation, problem }
    }

    // ── Acting ──

    fn say(&mut self, text: &str, warning: bool) {
        self.message = Some((text.to_string(), MESSAGE_SECONDS, warning));
    }

    fn use_item(&mut self) {
        match self.aim {
            Aim::Nothing => match self.selected {
                Item::Cable => self.say("Ziele auf einen Anschluss, um ein Kabel zu ziehen", true),
                Item::Device(_) => self.say("Ziele auf den Boden", true),
            },
            Aim::Port(end) => {
                let point = self.devices[end.device].port(end.port);
                self.drawing = Some(Drawing { from: end, points: vec![point], sags: Vec::new() });
                let text = format!("Kabel am {} angeschlossen", self.devices[end.device].kind.name());
                self.say(&text, false);
            }
            Aim::Next(NextPoint { problem: Some(problem), .. }) => self.say(problem, true),
            Aim::Next(next) => {
                let Some(mut drawing) = self.drawing.take() else {
                    return;
                };
                let last = *drawing.points.last().unwrap();
                drawing.sags.push(sag_for(last, next.point, self.sag_share));
                drawing.points.push(next.point);
                match next.end {
                    Some(to) => {
                        let cable = Cable { from: drawing.from, to, points: drawing.points, sags: drawing.sags, overhead: false };
                        let text = format!(
                            "Verbunden: {} – {} ({:.1} m)",
                            self.devices[cable.from.device].kind.name(),
                            self.devices[to.device].kind.name(),
                            cable.length()
                        );
                        self.cables.push(cable);
                        self.cable_models.push(None);
                        self.say(&text, false);
                    }
                    None => self.drawing = Some(drawing),
                }
            }
            Aim::Place { problem: Some(problem), .. } => self.say(problem, true),
            Aim::Place { kind, position, rotation, problem: None } => {
                self.place(kind, position, rotation);
                let text = format!("{} aufgestellt", kind.name());
                self.say(&text, false);
            }
        }
    }

    fn take_back(&mut self) {
        let Some(drawing) = &mut self.drawing else {
            return;
        };
        if drawing.points.len() > 1 {
            drawing.points.pop();
            drawing.sags.pop();
        } else {
            self.drawing = None;
            self.say("Kabel verworfen", true);
        }
    }

    fn toggle(&mut self) {
        let Some(Target::Device(d, _)) = self.target else {
            return;
        };
        let device = &mut self.devices[d];
        if !device.kind.switchable() {
            return;
        }
        device.on = !device.on;
        let text = format!("{} {}", device.kind.name(), if device.on { "an" } else { "aus" });
        self.say(&text, false);
    }

    fn take_down(&mut self) {
        match self.target {
            Some(Target::Cable(c)) => {
                self.remove_cable(c);
                self.say("Kabel abgebaut", false);
            }
            Some(Target::Device(d, _)) => {
                let text = format!("{} abgebaut", self.devices[d].kind.name());
                self.remove_device(d);
                self.say(&text, false);
            }
            None => {}
        }
        self.target = None;
    }

    // ── Moving and looking ──

    /// What the player stands on under `eye`: anything solid, or the
    /// terrain's surface while its chunk has no collider yet.
    fn floor_under(&self, physics: &mut PhysicsEngine, eye: Vector3<f32>) -> f32 {
        let from = eye + Vector3::new(0.0, 0.3, 0.0);
        match cast(physics, from, -Vector3::unit_y(), 60.0, &[]) {
            Some(hit) => from.y - hit.distance,
            None => self.ground_height(eye.x, eye.z),
        }
    }

    fn update_camera(&mut self, scene: &mut Scene, delta_time: f32) {
        let eye = match self.eye {
            Some(eye) => eye,
            None => {
                self.yaw = self.start_yaw.to_radians();
                self.pitch = -0.1;
                let [x, z] = self.start;
                let eye = Vector3::new(x, self.ground_height(x, z) + EYE_HEIGHT, z);
                self.eye = Some(eye);
                eye
            }
        };
        let (dx, dy) = std::mem::take(&mut self.look);
        let turn = self.mouse_sensitivity.to_radians();
        self.yaw += dx * turn;
        self.pitch = (self.pitch - dy * turn).clamp(-1.5, 1.5);
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
        let (_, physics) = scene.entities_and_physics_mut();
        if self.flying {
            eye += wish * self.fly_speed * delta_time;
            eye.y += axis(k.up, k.down) * self.fly_speed * delta_time;
            let floor = self.floor_under(physics, eye) + 0.3;
            eye.y = eye.y.max(floor);
            self.vertical_speed = 0.0;
        } else {
            eye += wish * self.walk_speed * delta_time;
            let floor = self.floor_under(physics, eye) + EYE_HEIGHT;
            let on_ground = eye.y <= floor + 0.05;
            if on_ground && std::mem::take(&mut self.jump) {
                self.vertical_speed = JUMP_SPEED;
            }
            self.vertical_speed -= GRAVITY * delta_time;
            eye.y += self.vertical_speed * delta_time;
            if eye.y < floor {
                eye.y = floor;
                self.vertical_speed = 0.0;
            }
        }
        self.jump = false;
        self.eye = Some(eye);
        let Some(camera) = scene.get_component_mut::<CameraComponent>() else {
            return;
        };
        camera.get_camera_controller_mut().is_free = false;
        let camera = camera.get_camera_mut();
        camera.set_relative_position(Point3::new(0.0, 0.0, 0.0));
        camera.set_position(Point3::from_vec(eye));
        camera.set_yaw_pitch(Rad(self.yaw), Rad(self.pitch));
    }

    // ── Drawing ──

    /// Make the meshes, once (needs a GPU), and the cables' tubes.
    fn make_models(&mut self) {
        if headless::is_headless() {
            return;
        }
        if !self.models_made {
            self.models_made = true;
            let build = |mesh: &ferrite::core::model::generated::MeshPart, color: [f32; 3]| -> Option<Model> {
                match generated::build_model(&[("part", mesh)]) {
                    Ok(mut model) => {
                        for material in 0..model.material_count().max(1) {
                            model.set_material_color(material, color);
                        }
                        Some(model)
                    }
                    Err(e) => {
                        ferrite::log::warn!("electrician: {e}");
                        None
                    }
                }
            };
            for kind in DeviceKind::ALL {
                let models = parts(kind)
                    .into_iter()
                    .filter_map(|p| build(&p.mesh, p.color).map(|m| (RefCell::new(m), p.glow, p.rotor)))
                    .collect();
                self.device_models.insert(kind, models);
            }
            for look in BlockLook::ALL {
                if let Some(model) = build(&devices::unit_cube(), look.color()) {
                    self.block_models.insert(look, RefCell::new(model));
                }
            }
            self.piece_model = build(&devices::cable_piece(), [0.07, 0.07, 0.08]).map(RefCell::new);
            self.ball_model = build(&devices::unit_ball(), [1.0, 1.0, 1.0]).map(RefCell::new);
        }
        self.cable_models.resize_with(self.cables.len(), || None);
        for (cable, model) in self.cables.iter().zip(self.cable_models.iter_mut()) {
            if model.is_some() {
                continue;
            }
            let mut mesh = ferrite::core::model::generated::MeshPart::default();
            let radius = if cable.overhead { OVERHEAD_RADIUS } else { CABLE_RADIUS };
            for span in cable.spans() {
                devices::tube(&mut mesh, &span.points(span.pieces()), radius, 6);
            }
            // A clip at every point it is fastened at.
            for p in &cable.points[1..cable.points.len() - 1] {
                devices::cuboid(&mut mesh, *p, Vector3::new(0.035, 0.035, 0.035), Quaternion::new(1.0, 0.0, 0.0, 0.0));
            }
            let color = if cable.overhead { [0.20, 0.20, 0.21] } else { [0.06, 0.06, 0.07] };
            if let Ok(mut built) = generated::build_model(&[("cable", &mesh)]) {
                for material in 0..built.material_count().max(1) {
                    built.set_material_color(material, color);
                }
                *model = Some(RefCell::new(built));
            }
        }
    }

    /// The part's transform on `device` (a rotor turns about its hub).
    fn part_transform(device: &Device, rotor: bool) -> Matrix4<f32> {
        if rotor {
            device.transform() * Matrix4::from_translation(devices::WIND_HUB) * Matrix4::from(Quaternion::from_angle_z(Rad(device.rotor)))
        } else {
            device.transform()
        }
    }

    /// How a glowing part looks now, if it glows.
    fn glow_tint(device: &Device, glow: Glow) -> Option<[f32; 4]> {
        let g = device.glow();
        let tint = match glow {
            Glow::None => return None,
            Glow::Bulb => match device.kind {
                DeviceKind::Floodlight => [7.0, 7.2, 7.5],
                DeviceKind::Pole => [6.0, 4.6, 2.6],
                _ => [3.2, 2.5, 1.6],
            }
            .map(|c| c * g),
            Glow::Coil => [5.0, 1.1, 0.25].map(|c| c * g),
            Glow::Led => {
                if g > 0.5 {
                    [0.3, 3.0, 0.4]
                } else {
                    return None;
                }
            }
            Glow::Gauge(n) => {
                let full = device.charge_wh / device.kind.capacity_wh().max(1.0);
                if full * 5.0 > n as f32 + 0.25 {
                    if full < 0.2 {
                        [3.0, 0.6, 0.2]
                    } else {
                        [0.4, 2.4, 0.6]
                    }
                } else {
                    return None;
                }
            }
        };
        (tint.iter().any(|c| *c > 0.05)).then_some([tint[0], tint[1], tint[2], 1.0])
    }

    fn draw_device(&self, device: &Device, tint: Option<[f32; 4]>, glowing: bool, lighting: &Lighting, view_projection: &Matrix4<f32>) {
        let Some(models) = self.device_models.get(&device.kind) else {
            return;
        };
        for (model, glow, rotor) in models {
            let transform = Self::part_transform(device, *rotor);
            let part_tint = match tint {
                Some(t) => Some(t),
                None => {
                    let lit = Self::glow_tint(device, *glow);
                    if glowing != lit.is_some() {
                        continue;
                    }
                    lit
                }
            };
            if part_tint.is_some() {
                model.borrow_mut().set_tint(part_tint);
            }
            model.borrow().render(lighting, &transform, view_projection);
            if part_tint.is_some() {
                model.borrow_mut().set_tint(None);
            }
        }
    }

    /// The cable being drawn: its spans so far and the next one, as
    /// pieces of the unit cylinder.
    fn draw_spans(&self, spans: &[Span], tint: Option<[f32; 4]>, radius: f32, lighting: &Lighting, view_projection: &Matrix4<f32>) {
        let Some(model) = &self.piece_model else {
            return;
        };
        if tint.is_some() {
            model.borrow_mut().set_tint(tint);
        }
        for span in spans {
            let points = span.points(span.pieces());
            for w in points.windows(2) {
                if let Some(transform) = piece_transform(w[0], w[1], radius) {
                    model.borrow().render(lighting, &transform, view_projection);
                }
            }
        }
        if tint.is_some() {
            model.borrow_mut().set_tint(None);
        }
    }

    fn draw_ball(&self, at: Vector3<f32>, radius: f32, tint: [f32; 4], lighting: &Lighting, view_projection: &Matrix4<f32>) {
        let Some(model) = &self.ball_model else {
            return;
        };
        model.borrow_mut().set_tint(Some(tint));
        let transform = Matrix4::from_translation(at) * Matrix4::from_scale(radius);
        model.borrow().render(lighting, &transform, view_projection);
        model.borrow_mut().set_tint(None);
    }

    fn drawing_spans(&self) -> Vec<Span> {
        self.drawing
            .as_ref()
            .map(|d| d.points.windows(2).zip(&d.sags).map(|(w, sag)| Span { a: w[0], b: w[1], sag: *sag }).collect())
            .unwrap_or_default()
    }

    // ── The HUD ──

    fn weather_name(&self) -> String {
        let sky = &self.sky;
        if sky.snow > 0.05 {
            "Schneefall".into()
        } else if sky.rain > 0.05 {
            if sky.rain > 0.7 {
                "Starkregen".into()
            } else {
                "Regen".into()
            }
        } else {
            match sky.clouds {
                c if c < 0.15 => "Klar".into(),
                c if c < 0.45 => "Leicht bewölkt".into(),
                c if c < 0.75 => "Bewölkt".into(),
                _ => "Bedeckt".into(),
            }
        }
    }

    fn device_panel(&self, d: usize, port: Option<usize>) -> Panel {
        let device = &self.devices[d];
        let kind = device.kind;
        let mut lines = Vec::new();
        let mut bar = None;
        match kind {
            DeviceKind::Solar | DeviceKind::Wind | DeviceKind::Generator => {
                lines.push((format!("Erzeugt {}  ·  möglich {}", kw(device.flow.produced_w), kw(device.available_w)), power_hud::POWER));
                lines.push((format!("Nennleistung {}", kw(kind.watts())), power_hud::TEXT_DIM));
                match kind {
                    DeviceKind::Solar => {
                        let normal = device.rotation * devices::solar_normal();
                        let sun = normal.dot(self.sky.sun).max(0.0) * sources::daylight(self.sky.sun);
                        lines.push((format!("Sonne {:.0} %  ·  Wetter {:.0} %", sun * 100.0, sources::weather_factor(&self.sky) * 100.0), power_hud::TEXT_DIM));
                        let shade = 1.0 - device.lit;
                        let color = if shade > 0.05 { power_hud::WARN } else { power_hud::TEXT_DIM };
                        lines.push((format!("Im Schatten {:.0} %", shade * 100.0), color));
                    }
                    DeviceKind::Wind => {
                        let state = if self.wind > sources::CUT_OUT {
                            "Sturm: abgeschaltet"
                        } else if self.wind < sources::CUT_IN {
                            "zu wenig Wind"
                        } else {
                            "dreht"
                        };
                        lines.push((format!("Wind {:.1} m/s  ·  {}", self.wind, state), power_hud::TEXT_DIM));
                    }
                    _ => lines.push((
                        if device.on { "Läuft  ·  E: ausschalten".into() } else { "Aus  ·  E: starten".into() },
                        if device.on { power_hud::GOOD } else { power_hud::TEXT_DIM },
                    )),
                }
                bar = Some((device.flow.produced_w / kind.watts(), power_hud::POWER));
            }
            DeviceKind::Battery => {
                let full = device.charge_wh / kind.capacity_wh();
                lines.push((format!("{:.2} / {:.1} kWh  ({:.0} %)", device.charge_wh / 1000.0, kind.capacity_wh() / 1000.0, full * 100.0), power_hud::STORE));
                let w = device.flow.battery_w;
                lines.push(if w > 1.0 {
                    (format!("Lädt mit {}", kw(w)), power_hud::GOOD)
                } else if w < -1.0 {
                    (format!("Gibt {} ab", kw(-w)), power_hud::WARN)
                } else {
                    ("Ruht".into(), power_hud::TEXT_DIM)
                });
                lines.push((format!("Laden/Entladen bis {}", kw(kind.watts())), power_hud::TEXT_DIM));
                bar = Some((full, power_hud::STORE));
            }
            DeviceKind::Pole => {
                let lamp = if device.demand_w > 0.0 {
                    if device.flow.supplied > 0.99 {
                        ("Straßenlaterne an (nachts)".to_string(), power_hud::GOOD)
                    } else if device.flow.supplied > 0.0 {
                        (format!("Straßenlaterne gedimmt ({:.0} %)", device.flow.supplied * 100.0), power_hud::WARN)
                    } else {
                        ("Straßenlaterne: kein Strom".to_string(), power_hud::WARN)
                    }
                } else {
                    ("Straßenlaterne aus (tagsüber)".to_string(), power_hud::TEXT_DIM)
                };
                lines.push(lamp);
                lines.push((format!("Laterne {}", kw(kind.watts())), power_hud::TEXT_DIM));
            }
            _ => {
                let (status, color) = if !device.on {
                    ("aus".to_string(), power_hud::TEXT_DIM)
                } else if device.flow.supplied > 0.99 {
                    ("versorgt".to_string(), power_hud::GOOD)
                } else if device.flow.supplied > 0.0 {
                    (format!("zu wenig Strom ({:.0} %)", device.flow.supplied * 100.0), power_hud::WARN)
                } else {
                    ("kein Strom".to_string(), power_hud::WARN)
                };
                lines.push((format!("{}  ·  {}", kw(kind.watts()), status), color));
                lines.push(("E: ein/aus".into(), power_hud::TEXT_DIM));
            }
        }
        let cables = self.cables.iter().filter(|c| c.joins(d)).count();
        let ports = match port {
            Some(p) => format!("{}  ·  {} Kabel", kind.port_name(p), cables),
            None => format!("{} Kabel angeschlossen", cables),
        };
        lines.push((ports, power_hud::TEXT_DIM));
        Panel { title: kind.name().to_string(), lines, bar }
    }

    fn network_panel(&self, network: usize) -> Option<Panel> {
        let net = self.networks.get(network)?;
        if net.devices < 2 {
            return None;
        }
        let mut lines = vec![
            (format!("Erzeugung {}  (möglich {})", kw(net.produced_w), kw(net.available_w)), power_hud::POWER),
            (
                format!("Verbrauch {}  ·  versorgt {:.0} %", kw(net.demand_w), net.supply() * 100.0),
                if net.supply() > 0.99 { power_hud::GOOD } else { power_hud::WARN },
            ),
        ];
        let mut bar = None;
        if net.capacity_wh > 0.0 {
            let flow = if net.battery_w > 1.0 {
                format!("  ·  +{}", kw(net.battery_w))
            } else if net.battery_w < -1.0 {
                format!("  ·  −{}", kw(-net.battery_w))
            } else {
                String::new()
            };
            lines.push((format!("Batterien {:.2} / {:.1} kWh{}", net.stored_wh / 1000.0, net.capacity_wh / 1000.0, flow), power_hud::STORE));
            bar = Some((net.stored_wh / net.capacity_wh, power_hud::STORE));
        }
        Some(Panel { title: format!("Stromnetz  ·  {} Geräte", net.devices), lines, bar })
    }
}

impl Behaviour for Electrician {
    fn update(&mut self, scene: &mut Scene, entity: &mut Entity, delta_time: f64) {
        self.playing = true;
        let dt = (delta_time as f32).min(0.1);
        self.clock += dt;
        if self.source.is_none() {
            self.source = scene.get_component::<Terrain>().and_then(|t| t.source().cloned());
        }
        if !self.set_up && self.source.is_some() {
            self.set_up_world();
        }
        self.make_models();
        self.read_sky(scene, dt);
        self.update_camera(scene, dt);

        let (_, physics) = scene.entities_and_physics_mut();
        self.sync_colliders(physics);
        self.shadow_timer -= dt;
        if self.shadow_timer <= 0.0 {
            self.shadow_timer = SHADOW_INTERVAL;
            self.find_shadows(physics);
        }
        self.compute_aim(physics);
        let pending = std::mem::take(&mut self.pending);
        if pending.scroll != 0.0 {
            if self.drawing.is_some() {
                // Up pulls the cable tighter.
                self.sag_share = (self.sag_share - pending.scroll * cable::SAG_STEP).clamp(0.0, cable::MAX_SAG_SHARE);
            } else {
                let i = self.selected.index() as i32 - pending.scroll.signum() as i32;
                let n = Item::ALL.len() as i32;
                self.select(Item::ALL[i.rem_euclid(n) as usize]);
            }
            self.compute_aim(physics);
        }
        for _ in 0..pending.clicks {
            self.use_item();
            self.compute_aim(physics);
        }
        for _ in 0..pending.right_clicks {
            self.take_back();
            self.compute_aim(physics);
        }
        if pending.toggle {
            self.toggle();
        }
        if pending.remove {
            self.take_down();
            self.sync_colliders(physics);
            self.compute_aim(physics);
        }
        self.run_grid(dt);
        self.sync_lights(entity);
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
        for block in &self.blocks {
            if let Some(model) = self.block_models.get(&block.look) {
                model.borrow().render(&lighting, &block.transform(), view_projection);
            }
        }
        for device in &self.devices {
            self.draw_device(device, None, false, &lighting, view_projection);
        }
        let identity = Matrix4::from_scale(1.0);
        for model in self.cable_models.iter().flatten() {
            model.borrow().render(&lighting, &identity, view_projection);
        }
        self.draw_spans(&self.drawing_spans(), None, CABLE_RADIUS, &lighting, view_projection);
    }

    fn render_transparent(&self, scene: &Scene, _entity: &Entity, view_projection: &Matrix4<f32>, _parent: &Matrix4<f32>) {
        let Some(lighting) = Lighting::from_scene(scene) else {
            return;
        };
        for device in &self.devices {
            self.draw_device(device, None, true, &lighting, view_projection);
        }
        match self.aim {
            Aim::Place { kind, position, rotation, problem } => {
                let preview = Device::new(kind, position, rotation);
                let tint = if problem.is_some() { PREVIEW_BAD } else { PREVIEW };
                self.draw_device(&preview, Some(tint), false, &lighting, view_projection);
            }
            Aim::Next(next) => {
                if let Some(last) = self.drawing.as_ref().and_then(|d| d.points.last()) {
                    let span = Span::with_share(*last, next.point, self.sag_share);
                    let tint = if next.problem.is_some() { CABLE_PREVIEW_BAD } else { CABLE_PREVIEW };
                    self.draw_spans(&[span], Some(tint), CABLE_RADIUS * 1.3, &lighting, view_projection);
                    if next.end.is_none() {
                        self.draw_ball(next.point, 0.05, tint, &lighting, view_projection);
                    }
                }
            }
            _ => {}
        }
        // The ports, while a cable is in hand.
        if self.selected == Item::Cable {
            let Some(eye) = self.eye else {
                return;
            };
            let aimed = match self.aim {
                Aim::Port(end) => Some(end),
                Aim::Next(next) => next.end,
                _ => None,
            };
            for (d, device) in self.devices.iter().enumerate() {
                for p in 0..device.kind.ports().len() {
                    let at = device.port(p);
                    if (at - eye).magnitude() > 25.0 {
                        continue;
                    }
                    let end = End { device: d, port: p };
                    let busy = self.cables.iter().any(|c| c.from == end || c.to == end);
                    let (tint, radius) = if aimed == Some(end) {
                        (PORT_AIMED, 0.09)
                    } else if busy {
                        (PORT_BUSY, 0.06)
                    } else {
                        (PORT, 0.065)
                    };
                    self.draw_ball(at, radius, tint, &lighting, view_projection);
                }
            }
        }
    }

    fn render_overlay(&self, _scene: &Scene, _entity: &Entity) {
        if !self.playing {
            return;
        }
        let target = match self.target {
            Some(Target::Device(d, port)) if d < self.devices.len() => Some(self.device_panel(d, port)),
            Some(Target::Cable(c)) if c < self.cables.len() => {
                let cable = &self.cables[c];
                let name = |end: End| self.devices.get(end.device).map_or("?", |d| d.kind.name());
                Some(Panel {
                    title: if cable.overhead { "Freileitung".into() } else { "Kabel".into() },
                    lines: vec![
                        (format!("{} – {}", name(cable.from), name(cable.to)), power_hud::TEXT),
                        (format!("{:.1} m  ·  {} Abschnitte", cable.length(), cable.sags.len()), power_hud::TEXT_DIM),
                        ("X: abbauen".into(), power_hud::TEXT_DIM),
                    ],
                    bar: None,
                })
            }
            _ => None,
        };
        let network = match (&self.drawing, self.target) {
            (Some(drawing), _) => self.devices.get(drawing.from.device).map(|d| d.network),
            (None, Some(Target::Device(d, _))) => self.devices.get(d).map(|d| d.network),
            (None, Some(Target::Cable(c))) => self.cables.get(c).and_then(|c| self.devices.get(c.from.device)).map(|d| d.network),
            _ => None,
        }
        .and_then(|n| self.network_panel(n));
        let next = match self.aim {
            Aim::Next(next) => Some(next),
            _ => None,
        };
        let cable_info = self.drawing.as_ref().map(|drawing| {
            let last = *drawing.points.last().unwrap();
            let preview = next.map_or(0.0, |n| Span::with_share(last, n.point, self.sag_share).length());
            CableInfo {
                length: self.drawn_length(drawing) + preview,
                max: self.cable_length,
                sag: next.map_or(0.0, |n| sag_for(last, n.point, self.sag_share)),
                lock: next.and_then(|n| n.lock).map(Lock::name),
                points: drawing.points.len() - 1,
                problem: next.and_then(|n| n.problem),
                from: self.devices.get(drawing.from.device).map_or("?", |d| d.kind.name()),
            }
        });
        let hint_text;
        let hint = match (self.selected, &self.drawing) {
            (Item::Cable, Some(_)) => "Klick: befestigen oder an Anschluss  ·  Mausrad: Durchhang  ·  Shift: senk-/waagerecht  ·  Rechtsklick: zurück",
            (Item::Cable, None) => "Klick auf einen Anschluss: Kabel ziehen  ·  E: ein/aus  ·  X: abbauen  ·  T: Zeitraffer  ·  G: Wetter",
            (Item::Device(kind), _) => {
                hint_text = format!("Klick: {} aufstellen  ·  R: drehen  ·  E: ein/aus  ·  X: abbauen  ·  T: Zeitraffer  ·  G: Wetter", kind.name());
                hint_text.as_str()
            }
        };
        let message = self.message.as_ref().map(|(text, time, warning)| (text.as_str(), time.min(0.5) * 2.0, *warning));
        let weather = self.weather_name();
        let sun = sources::daylight(self.sky.sun) * self.sky.sun.y.max(0.0).sqrt() * sources::weather_factor(&self.sky);
        self.hud.draw(&HudState {
            selected: self.selected,
            hint,
            message,
            target,
            network,
            cable: cable_info,
            clock: self.time_of_day,
            weather: &weather,
            wind: self.wind,
            sun,
            warp: self.warp,
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
                    Key::LeftControl => self.keys.down = down,
                    Key::LeftShift | Key::RightShift => self.shift = down,
                    _ if !down => {}
                    Key::Num1 | Key::Num2 | Key::Num3 | Key::Num4 | Key::Num5 | Key::Num6 | Key::Num7 | Key::Num8 | Key::Num9 => {
                        let i = *key as usize - Key::Num1 as usize;
                        self.select(Item::ALL[i]);
                    }
                    Key::E => self.pending.toggle = true,
                    Key::X => self.pending.remove = true,
                    Key::G => self.pending.weather = true,
                    Key::T => {
                        self.warp = !self.warp;
                        self.say(if self.warp { "Zeitraffer an" } else { "Zeitraffer aus" }, false);
                    }
                    Key::F => self.flying = !self.flying,
                    Key::R => self.turn += 15f32.to_radians(),
                    Key::Escape => {
                        self.release_cursor = !self.release_cursor;
                        window.set_cursor_mode(if self.release_cursor { glfw::CursorMode::Normal } else { glfw::CursorMode::Disabled });
                        self.last_cursor = None;
                    }
                    _ => {}
                }
            }
            WindowEvent::Scroll(_, y) if *y != 0.0 => self.pending.scroll += *y as f32,
            WindowEvent::MouseButton(button, Action::Press, _) => match button {
                MouseButton::Button1 => self.pending.clicks += 1,
                MouseButton::Button2 => self.pending.right_clicks += 1,
                _ => {}
            },
            WindowEvent::CursorPos(x, y) => {
                let captured = window.get_cursor_mode() == glfw::CursorMode::Disabled;
                if let (true, Some((lx, ly))) = (captured, self.last_cursor) {
                    self.look.0 += (x - lx) as f32;
                    self.look.1 += (y - ly) as f32;
                }
                self.last_cursor = Some((*x, *y));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: f32, y: f32, z: f32) -> Vector3<f32> {
        Vector3::new(x, y, z)
    }

    #[test]
    fn a_frame_stands_on_its_surface() {
        // On a wall facing +x, the device's up is +x.
        let q = frame(v(1.0, 0.0, 0.0), v(0.0, 0.0, 1.0));
        assert!((q * Vector3::unit_y() - v(1.0, 0.0, 0.0)).magnitude() < 1e-4);
        assert!((q * Vector3::unit_z() - v(0.0, 0.0, 1.0)).magnitude() < 1e-4);
        // On a ceiling, upside down.
        let q = frame(v(0.0, -1.0, 0.0), v(0.0, 0.0, 1.0));
        assert!((q * Vector3::unit_y() - v(0.0, -1.0, 0.0)).magnitude() < 1e-4);
    }

    #[test]
    fn upright_devices_face_the_player() {
        let q = upright(v(1.0, 0.0, 0.0), 0.0);
        assert!((q * Vector3::unit_z() - v(1.0, 0.0, 0.0)).magnitude() < 1e-4);
        let q = upright(v(0.0, 0.0, -1.0), FRAC_PI_2);
        assert!((q * Vector3::unit_z() - v(-1.0, 0.0, 0.0)).magnitude() < 1e-4);
    }

    #[test]
    fn taking_a_device_down_takes_its_cables_and_keeps_the_rest_joined() {
        let mut e = Electrician::default();
        let q = Quaternion::new(1.0, 0.0, 0.0, 0.0);
        let a = e.place(DeviceKind::Solar, v(0.0, 0.0, 0.0), q);
        let b = e.place(DeviceKind::Battery, v(3.0, 0.0, 0.0), q);
        let c = e.place(DeviceKind::Lamp, v(6.0, 0.0, 0.0), q);
        e.connect(End { device: a, port: 0 }, End { device: b, port: 0 }, &[], 0.03, false);
        e.connect(End { device: b, port: 0 }, End { device: c, port: 0 }, &[v(4.5, 1.0, 0.0)], 0.03, false);
        assert_eq!(e.cables().len(), 2);
        e.remove_device(a);
        assert_eq!(e.devices().len(), 2);
        assert_eq!(e.cables().len(), 1);
        // The battery and the lamp moved down by one.
        assert_eq!(e.cables()[0].from.device, 0);
        assert_eq!(e.cables()[0].to.device, 1);
        assert_eq!(e.cables()[0].points.len(), 3);
    }

    #[test]
    fn the_grid_runs_over_cables() {
        let mut e = Electrician::default();
        let q = Quaternion::new(1.0, 0.0, 0.0, 0.0);
        let generator = e.place(DeviceKind::Generator, v(0.0, 0.0, 0.0), q);
        let lamp = e.place(DeviceKind::Lamp, v(3.0, 0.0, 0.0), q);
        let alone = e.place(DeviceKind::Lamp, v(9.0, 0.0, 0.0), q);
        e.connect(End { device: generator, port: 0 }, End { device: lamp, port: 0 }, &[], 0.03, false);
        e.run_grid(0.1);
        assert_eq!(e.devices()[lamp].flow.supplied, 1.0);
        assert_eq!(e.devices()[alone].flow.supplied, 0.0);
        assert!((e.devices()[generator].flow.produced_w - 60.0).abs() < 1e-3);
        // Switched off, the generator makes nothing.
        e.device_mut(generator).on = false;
        e.run_grid(0.1);
        assert_eq!(e.devices()[lamp].flow.supplied, 0.0);
    }
}
