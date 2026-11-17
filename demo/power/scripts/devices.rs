//! The devices of the power demo: what each one is, how many watts it makes
//! or uses, where its ports are, what it looks like (meshes made in code,
//! plain colours) and the boxes it collides with.
//!
//! Every device is modelled in its own frame: +Y is up (the surface it
//! stands on), +Z its front. A device's ports are all one node of the grid:
//! a cable on any of them joins the same network.

use std::f32::consts::{FRAC_PI_2, TAU};

use ferrite::core::model::generated::MeshPart;
use ferrite::prelude::*;

/// What a device does in the grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Producer,
    Consumer,
    Storage,
    /// Only carries power on (a pole); its street lamp uses some.
    Line,
}

/// Where a device can be put.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mount {
    /// On ground or a floor: something you can stand on.
    Floor,
    /// On any surface: a floor, a wall, a ceiling.
    Any,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DeviceKind {
    Solar,
    Wind,
    Generator,
    Battery,
    Lamp,
    Floodlight,
    Heater,
    Pole,
}

impl DeviceKind {
    pub const ALL: [DeviceKind; 8] = [
        DeviceKind::Solar,
        DeviceKind::Wind,
        DeviceKind::Generator,
        DeviceKind::Battery,
        DeviceKind::Lamp,
        DeviceKind::Floodlight,
        DeviceKind::Heater,
        DeviceKind::Pole,
    ];

    pub fn name(self) -> &'static str {
        match self {
            DeviceKind::Solar => "Solarpanel",
            DeviceKind::Wind => "Windrad",
            DeviceKind::Generator => "Generator",
            DeviceKind::Battery => "Batterie",
            DeviceKind::Lamp => "Lampe",
            DeviceKind::Floodlight => "Strahler",
            DeviceKind::Heater => "Heizlüfter",
            DeviceKind::Pole => "Strommast",
        }
    }

    pub fn role(self) -> Role {
        match self {
            DeviceKind::Solar | DeviceKind::Wind | DeviceKind::Generator => Role::Producer,
            DeviceKind::Battery => Role::Storage,
            DeviceKind::Lamp | DeviceKind::Floodlight | DeviceKind::Heater => Role::Consumer,
            DeviceKind::Pole => Role::Line,
        }
    }

    /// Rated watts: the most a producer makes, what a consumer uses while
    /// on, how fast a battery charges and discharges, what a pole's street
    /// lamp uses at night.
    pub fn watts(self) -> f32 {
        match self {
            DeviceKind::Solar => 400.0,
            DeviceKind::Wind => 3000.0,
            DeviceKind::Generator => 2500.0,
            DeviceKind::Battery => 3000.0,
            DeviceKind::Lamp => 60.0,
            DeviceKind::Floodlight => 400.0,
            DeviceKind::Heater => 2000.0,
            DeviceKind::Pole => 100.0,
        }
    }

    /// What a battery holds when full, watt hours.
    pub fn capacity_wh(self) -> f32 {
        match self {
            DeviceKind::Battery => 5000.0,
            _ => 0.0,
        }
    }

    /// Whether `E` switches it on and off.
    pub fn switchable(self) -> bool {
        matches!(
            self,
            DeviceKind::Generator | DeviceKind::Lamp | DeviceKind::Floodlight | DeviceKind::Heater
        )
    }

    pub fn mount(self) -> Mount {
        match self {
            DeviceKind::Lamp => Mount::Any,
            _ => Mount::Floor,
        }
    }

    /// How far (horizontally) another device has to stay away.
    pub fn footprint(self) -> f32 {
        match self {
            DeviceKind::Solar => 0.95,
            DeviceKind::Wind => 0.6,
            DeviceKind::Generator => 0.7,
            DeviceKind::Battery => 0.35,
            DeviceKind::Lamp => 0.12,
            DeviceKind::Floodlight => 0.3,
            DeviceKind::Heater => 0.3,
            DeviceKind::Pole => 0.4,
        }
    }

    /// The ports, in the device's frame. Cables start and end there.
    pub fn ports(self) -> &'static [[f32; 3]] {
        match self {
            DeviceKind::Solar => &[[0.0, 0.47, -0.36]],
            DeviceKind::Wind => &[[0.47, 0.32, 0.0]],
            DeviceKind::Generator => &[[0.2, 0.42, 0.39]],
            DeviceKind::Battery => &[[0.15, 1.31, 0.0]],
            DeviceKind::Lamp => &[[0.1, 0.02, 0.0]],
            DeviceKind::Floodlight => &[[0.2, 0.07, -0.2]],
            DeviceKind::Heater => &[[0.2, 0.15, -0.14]],
            // The box at the bottom (to branch off), the two insulators at
            // the top (for the overhead line).
            DeviceKind::Pole => &[[0.0, 1.42, 0.21], [0.0, 7.76, 0.75], [0.0, 7.76, -0.75]],
        }
    }

    /// The name of port `i`, for the HUD.
    pub fn port_name(self, i: usize) -> &'static str {
        match (self, i) {
            (DeviceKind::Pole, 0) => "Abzweigkasten",
            (DeviceKind::Pole, _) => "Isolator",
            (DeviceKind::Battery, _) => "Pol",
            _ => "Anschluss",
        }
    }

    /// Where its light shines from and which way (device frame), if it
    /// makes light.
    pub fn light_anchor(self) -> Option<(Vector3<f32>, Vector3<f32>)> {
        match self {
            DeviceKind::Lamp => Some((Vector3::new(0.0, 0.34, 0.0), Vector3::unit_y())),
            DeviceKind::Floodlight => {
                let tilt = floodlight_tilt();
                Some((Vector3::new(0.0, 1.25, 0.05) + tilt * Vector3::new(0.0, 0.0, 0.12), tilt * Vector3::unit_z()))
            }
            DeviceKind::Heater => Some((Vector3::new(0.0, 0.3, 0.3), Vector3::unit_z())),
            DeviceKind::Pole => Some((Vector3::new(0.0, 6.42, 1.5), -Vector3::unit_y())),
            _ => None,
        }
    }

    /// The boxes it collides with: centre, half size, rotation (device
    /// frame). Cables can be fastened to them, and they cast shadows on
    /// solar panels.
    pub fn boxes(self) -> Vec<(Vector3<f32>, Vector3<f32>, Quaternion<f32>)> {
        let q = Quaternion::new(1.0, 0.0, 0.0, 0.0);
        let v = Vector3::new;
        match self {
            DeviceKind::Solar => vec![(SOLAR_CENTRE, v(0.85, 0.03, 0.5), solar_tilt()), (v(0.0, 0.4, 0.0), v(0.78, 0.4, 0.42), q)],
            DeviceKind::Wind => vec![
                (v(0.0, 0.15, 0.0), v(0.45, 0.15, 0.45), q),
                (v(0.0, 4.15, 0.0), v(0.12, 3.85, 0.12), q),
                (v(0.0, WIND_HUB.y, -0.15), v(0.18, 0.18, 0.45), q),
            ],
            DeviceKind::Generator => vec![(v(0.0, 0.47, 0.0), v(0.62, 0.47, 0.37), q)],
            DeviceKind::Battery => vec![(v(0.0, 0.62, 0.0), v(0.32, 0.62, 0.22), q)],
            DeviceKind::Lamp => vec![(v(0.0, 0.2, 0.0), v(0.1, 0.2, 0.1), q)],
            DeviceKind::Floodlight => vec![(v(0.0, 0.65, 0.0), v(0.22, 0.65, 0.22), q)],
            DeviceKind::Heater => vec![(v(0.0, 0.3, 0.0), v(0.3, 0.3, 0.13), q)],
            DeviceKind::Pole => vec![
                (v(0.0, 4.0, 0.0), v(0.13, 4.0, 0.13), q),
                (v(0.0, 7.5, 0.0), v(0.06, 0.06, 0.9), q),
                (v(0.0, 7.66, 0.75), v(0.05, 0.1, 0.05), q),
                (v(0.0, 7.66, -0.75), v(0.05, 0.1, 0.05), q),
                (v(0.0, 1.7, 0.21), v(0.18, 0.25, 0.08), q),
            ],
        }
    }
}

/// The middle of a solar panel's face, device frame.
pub const SOLAR_CENTRE: Vector3<f32> = Vector3::new(0.0, 1.0, 0.0);
/// Half the size of a solar panel's face (along its width and its slope).
pub const SOLAR_HALF: [f32; 2] = [0.85, 0.5];
/// Where a wind turbine's rotor turns, device frame.
pub const WIND_HUB: Vector3<f32> = Vector3::new(0.0, 8.15, 0.36);

/// How a solar panel leans toward its front.
pub fn solar_tilt() -> Quaternion<f32> {
    Quaternion::from_angle_x(Deg(25.0))
}

/// Which way a solar panel's face looks, device frame.
pub fn solar_normal() -> Vector3<f32> {
    solar_tilt() * Vector3::unit_y()
}

/// How a floodlight's head points down toward its front.
pub fn floodlight_tilt() -> Quaternion<f32> {
    Quaternion::from_angle_x(Deg(20.0))
}

// ── Meshes ──

/// Which parts light up, and when.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Glow {
    None,
    /// A lamp's bulb or a floodlight's lens: bright while powered.
    Bulb,
    /// A heater's coils: glowing red while powered.
    Coil,
    /// A generator's status light: green while running.
    Led,
    /// Segment `n` (from the bottom) of a battery's charge gauge.
    Gauge(u8),
}

/// One coloured part of a device's look.
pub struct Part {
    pub mesh: MeshPart,
    /// Plain colour, sRGB.
    pub color: [f32; 3],
    pub glow: Glow,
    /// Turns with the wind (a turbine's rotor): its mesh is around the
    /// hub, turning about its Z axis.
    pub rotor: bool,
}

fn part(color: [f32; 3], build: impl FnOnce(&mut MeshPart)) -> Part {
    let mut mesh = MeshPart::default();
    build(&mut mesh);
    Part { mesh, color, glow: Glow::None, rotor: false }
}

fn glowing(glow: Glow, color: [f32; 3], build: impl FnOnce(&mut MeshPart)) -> Part {
    Part { glow, ..part(color, build) }
}

const STEEL: [f32; 3] = [0.62, 0.64, 0.66];
const DARK: [f32; 3] = [0.12, 0.12, 0.13];
const GREY: [f32; 3] = [0.42, 0.43, 0.45];
const WHITE: [f32; 3] = [0.86, 0.87, 0.86];

/// The look of a device, part by part.
pub fn parts(kind: DeviceKind) -> Vec<Part> {
    let v = Vector3::new;
    let q = Quaternion::new(1.0, 0.0, 0.0, 0.0);
    match kind {
        DeviceKind::Solar => {
            let tilt = solar_tilt();
            let on_panel = |x: f32, z: f32, y: f32| SOLAR_CENTRE + tilt * v(x, y, z);
            let [hw, hd] = SOLAR_HALF;
            vec![
                part([0.06, 0.10, 0.24], |m| cuboid(m, SOLAR_CENTRE, v(hw - 0.03, 0.03, hd - 0.03), tilt)),
                part(STEEL, |m| {
                    // The frame around the cells.
                    cuboid(m, on_panel(0.0, hd - 0.015, 0.0), v(hw, 0.035, 0.015), tilt);
                    cuboid(m, on_panel(0.0, -hd + 0.015, 0.0), v(hw, 0.035, 0.015), tilt);
                    cuboid(m, on_panel(hw - 0.015, 0.0, 0.0), v(0.015, 0.035, hd), tilt);
                    cuboid(m, on_panel(-hw + 0.015, 0.0, 0.0), v(0.015, 0.035, hd), tilt);
                    // The stand: two legs at each side, rails on the ground.
                    for x in [-0.72f32, 0.72] {
                        let front = on_panel(x, hd - 0.06, -0.03);
                        let back = on_panel(x, -hd + 0.06, -0.03);
                        cylinder(m, v(x, 0.04, front.z), front, 0.025, 6);
                        cylinder(m, v(x, 0.04, back.z), back, 0.025, 6);
                        cuboid(m, v(x, 0.02, 0.0), v(0.03, 0.02, 0.5), q);
                    }
                    cylinder(m, on_panel(-0.72, 0.0, -0.05), on_panel(0.72, 0.0, -0.05), 0.02, 6);
                }),
                part([0.50, 0.56, 0.68], |m| {
                    // Lines between the cells.
                    for i in 1..6 {
                        let x = -hw + 0.03 + i as f32 * (2.0 * (hw - 0.03)) / 6.0;
                        cuboid(m, on_panel(x, 0.0, 0.032), v(0.004, 0.002, hd - 0.04), tilt);
                    }
                    for i in 1..4 {
                        let z = -hd + 0.03 + i as f32 * (2.0 * (hd - 0.03)) / 4.0;
                        cuboid(m, on_panel(0.0, z, 0.032), v(hw - 0.04, 0.002, 0.004), tilt);
                    }
                }),
                part(DARK, |m| {
                    // The junction box under the panel, with its port.
                    cuboid(m, v(0.0, 0.56, -0.36), v(0.09, 0.07, 0.05), q);
                    cylinder(m, v(0.0, 0.49, -0.36), v(0.0, 0.45, -0.36), 0.025, 8);
                }),
            ]
        }
        DeviceKind::Wind => {
            let hub = WIND_HUB;
            let blades: Vec<Quaternion<f32>> = (0..3).map(|k| Quaternion::from_angle_z(Rad(k as f32 * TAU / 3.0))).collect();
            vec![
                part([0.55, 0.55, 0.53], |m| cuboid(m, v(0.0, 0.15, 0.0), v(0.45, 0.15, 0.45), q)),
                part(WHITE, |m| {
                    cylinder(m, v(0.0, 0.3, 0.0), v(0.0, 4.0, 0.0), 0.13, 12);
                    cylinder(m, v(0.0, 4.0, 0.0), v(0.0, hub.y - 0.1, 0.0), 0.1, 12);
                    cuboid(m, v(0.0, hub.y, -0.15), v(0.17, 0.17, 0.45), q);
                    // The tail vane.
                    cuboid(m, v(0.0, hub.y + 0.1, -0.95), v(0.015, 0.28, 0.32), q);
                    cylinder(m, v(0.0, hub.y, -0.55), v(0.0, hub.y + 0.05, -0.7), 0.03, 6);
                }),
                part(GREY, |m| {
                    cuboid(m, v(0.35, 0.42, 0.0), v(0.08, 0.12, 0.1), q);
                    cylinder(m, v(0.43, 0.34, 0.0), v(0.47, 0.32, 0.0), 0.025, 8);
                }),
                Part {
                    rotor: true,
                    ..part([0.92, 0.92, 0.9], |m| {
                        cylinder(m, v(0.0, 0.0, -0.08), v(0.0, 0.0, 0.18), 0.13, 12);
                        cylinder(m, v(0.0, 0.0, 0.18), v(0.0, 0.0, 0.28), 0.07, 12);
                        for r in &blades {
                            // A blade: wide at the root, narrow at the tip,
                            // a little twisted.
                            for (i, (from, to, width)) in [(0.12f32, 0.8f32, 0.11f32), (0.8, 1.5, 0.08), (1.5, 2.1, 0.05)].into_iter().enumerate() {
                                let twist = Quaternion::from_angle_y(Deg(18.0 - i as f32 * 6.0));
                                cuboid(m, *r * v(0.0, (from + to) / 2.0, 0.05), v(width, (to - from) / 2.0, 0.012), *r * twist);
                            }
                        }
                    })
                },
            ]
        }
        DeviceKind::Generator => vec![
            part([0.88, 0.46, 0.10], |m| {
                cuboid(m, v(0.0, 0.46, 0.0), v(0.6, 0.34, 0.34), q);
            }),
            part([0.70, 0.34, 0.06], |m| cuboid(m, v(0.0, 0.85, 0.0), v(0.45, 0.06, 0.27), q)),
            part(DARK, |m| {
                cuboid(m, v(0.0, 0.06, 0.0), v(0.63, 0.06, 0.37), q);
                cylinder(m, v(-0.45, 0.8, -0.2), v(-0.45, 1.15, -0.2), 0.04, 8);
                cuboid(m, v(0.25, 0.5, 0.345), v(0.22, 0.16, 0.01), q);
                cylinder(m, v(0.2, 0.42, 0.35), v(0.2, 0.42, 0.39), 0.035, 10);
                // Vents on the side.
                for i in 0..5 {
                    cuboid(m, v(-0.3, 0.32 + i as f32 * 0.06, 0.342), v(0.22, 0.008, 0.006), q);
                }
            }),
            glowing(Glow::Led, [0.10, 0.25, 0.10], |m| cuboid(m, v(0.38, 0.6, 0.356), v(0.025, 0.025, 0.006), q)),
        ],
        DeviceKind::Battery => {
            let mut parts = vec![
                part([0.80, 0.82, 0.84], |m| cuboid(m, v(0.0, 0.64, 0.0), v(0.32, 0.58, 0.22), q)),
                part(DARK, |m| {
                    cuboid(m, v(0.0, 0.04, 0.0), v(0.33, 0.04, 0.23), q);
                    cuboid(m, v(0.0, 0.75, 0.222), v(0.15, 0.28, 0.006), q);
                }),
                part([0.22, 0.55, 0.30], |m| cuboid(m, v(0.0, 1.12, 0.223), v(0.32, 0.03, 0.006), q)),
                part([0.75, 0.15, 0.12], |m| cylinder(m, v(0.15, 1.22, 0.0), v(0.15, 1.3, 0.0), 0.04, 10)),
                part(DARK, |m| cylinder(m, v(-0.15, 1.22, 0.0), v(-0.15, 1.3, 0.0), 0.04, 10)),
            ];
            for i in 0..5u8 {
                parts.push(glowing(Glow::Gauge(i), [0.06, 0.16, 0.08], |m| {
                    cuboid(m, v(0.0, 0.55 + i as f32 * 0.1, 0.228), v(0.12, 0.035, 0.006), q)
                }));
            }
            parts
        }
        DeviceKind::Lamp => vec![
            part(DARK, |m| {
                cylinder(m, v(0.0, 0.0, 0.0), v(0.0, 0.03, 0.0), 0.09, 14);
                cylinder(m, v(0.0, 0.03, 0.0), v(0.0, 0.22, 0.0), 0.014, 8);
                cylinder(m, v(0.0, 0.2, 0.0), v(0.0, 0.25, 0.0), 0.05, 12);
            }),
            glowing(Glow::Bulb, [0.92, 0.90, 0.84], |m| sphere(m, v(0.0, 0.34, 0.0), 0.1, 8, 14)),
        ],
        DeviceKind::Floodlight => {
            let tilt = floodlight_tilt();
            let head = v(0.0, 1.25, 0.05);
            vec![
                part(DARK, |m| {
                    cuboid(m, v(0.0, 0.03, 0.0), v(0.25, 0.03, 0.25), q);
                    cylinder(m, v(0.0, 0.06, 0.0), v(0.0, 1.12, 0.0), 0.035, 8);
                    cuboid(m, v(0.0, 1.12, 0.0), v(0.24, 0.025, 0.04), q);
                    for x in [-0.23f32, 0.23] {
                        cuboid(m, v(x, 1.2, 0.0), v(0.015, 0.09, 0.03), q);
                    }
                    cylinder(m, v(0.2, 0.07, -0.2), v(0.2, 0.1, -0.2), 0.03, 8);
                }),
                part([0.28, 0.29, 0.31], |m| cuboid(m, head, v(0.2, 0.15, 0.08), tilt)),
                glowing(Glow::Bulb, [0.80, 0.82, 0.80], |m| cuboid(m, head + tilt * v(0.0, 0.0, 0.081), v(0.17, 0.12, 0.004), tilt)),
            ]
        }
        DeviceKind::Heater => {
            let mut parts = vec![
                part([0.90, 0.90, 0.88], |m| {
                    cuboid(m, v(0.0, 0.32, 0.0), v(0.3, 0.24, 0.12), q);
                    cuboid(m, v(0.0, 0.58, 0.0), v(0.08, 0.02, 0.04), q);
                }),
                part(DARK, |m| {
                    for x in [-0.22f32, 0.22] {
                        cuboid(m, v(x, 0.04, 0.0), v(0.04, 0.04, 0.14), q);
                    }
                    cuboid(m, v(0.0, 0.32, 0.118), v(0.25, 0.18, 0.004), q);
                    cylinder(m, v(0.2, 0.15, -0.12), v(0.2, 0.15, -0.14), 0.03, 8);
                }),
            ];
            parts.push(glowing(Glow::Coil, [0.30, 0.18, 0.14], |m| {
                for i in 0..4 {
                    cuboid(m, v(0.0, 0.2 + i as f32 * 0.08, 0.124), v(0.22, 0.012, 0.006), q);
                }
            }));
            parts
        }
        DeviceKind::Pole => vec![
            part([0.34, 0.24, 0.15], |m| {
                cylinder(m, v(0.0, 0.0, 0.0), v(0.0, 8.0, 0.0), 0.13, 12);
                cuboid(m, v(0.0, 7.5, 0.0), v(0.06, 0.06, 0.92), q);
                // Braces from the pole to the arm.
                for z in [-0.5f32, 0.5] {
                    cylinder(m, v(0.0, 7.0, 0.0), v(0.0, 7.46, z), 0.025, 6);
                }
            }),
            part([0.84, 0.88, 0.92], |m| {
                for z in [-0.75f32, 0.75] {
                    cylinder(m, v(0.0, 7.56, z), v(0.0, 7.6, z), 0.05, 10);
                    cylinder(m, v(0.0, 7.6, z), v(0.0, 7.64, z), 0.035, 10);
                    cylinder(m, v(0.0, 7.64, z), v(0.0, 7.68, z), 0.05, 10);
                    cylinder(m, v(0.0, 7.68, z), v(0.0, 7.75, z), 0.03, 10);
                }
            }),
            part([0.56, 0.58, 0.56], |m| {
                // The box to branch off from, and the conduit up the pole.
                cuboid(m, v(0.0, 1.7, 0.21), v(0.18, 0.25, 0.08), q);
                cylinder(m, v(0.0, 1.95, 0.16), v(0.0, 7.0, 0.16), 0.022, 6);
                cylinder(m, v(0.0, 1.45, 0.21), v(0.0, 1.41, 0.21), 0.03, 8);
                // The street lamp's arm and head.
                cylinder(m, v(0.0, 6.2, 0.12), v(0.0, 6.5, 1.3), 0.035, 8);
                cuboid(m, v(0.0, 6.5, 1.5), v(0.12, 0.05, 0.25), q);
            }),
            part([0.95, 0.75, 0.10], |m| cuboid(m, v(0.0, 1.82, 0.292), v(0.06, 0.06, 0.003), q)),
            glowing(Glow::Bulb, [0.75, 0.76, 0.74], |m| cuboid(m, v(0.0, 6.44, 1.5), v(0.09, 0.008, 0.19), q)),
        ],
    }
}

/// A unit cylinder along +Z from 0 to 1, radius 1: a piece of cable,
/// scaled to its length and thickness.
pub fn cable_piece() -> MeshPart {
    let mut m = MeshPart::default();
    cylinder(&mut m, Vector3::zero(), Vector3::unit_z(), 1.0, 8);
    m
}

/// A unit cube from -0.5 to 0.5: walls, floors and clips, scaled.
pub fn unit_cube() -> MeshPart {
    let mut m = MeshPart::default();
    cuboid(&mut m, Vector3::zero(), Vector3::new(0.5, 0.5, 0.5), Quaternion::new(1.0, 0.0, 0.0, 0.0));
    m
}

/// A small ball around the origin, radius 1: ports while a cable is drawn.
pub fn unit_ball() -> MeshPart {
    let mut m = MeshPart::default();
    sphere(&mut m, Vector3::zero(), 1.0, 6, 10);
    m
}

/// A round tube of `radius` through `points` (a cable), open at its ends.
pub fn tube(mesh: &mut MeshPart, points: &[Vector3<f32>], radius: f32, sides: usize) {
    if points.len() < 2 {
        return;
    }
    // Rings across the direction at each point, turned along with the
    // tube (each ring's frame follows from the last), so it does not
    // twist.
    let tangent = |i: usize| {
        let a = points[i.saturating_sub(1)];
        let b = points[(i + 1).min(points.len() - 1)];
        let d = b - a;
        if d.magnitude2() > 1e-12 {
            d.normalize()
        } else {
            Vector3::unit_x()
        }
    };
    let (mut e1, _) = across(tangent(0));
    let mut previous: Option<Vec<u32>> = None;
    for (i, p) in points.iter().enumerate() {
        let t = tangent(i);
        e1 = (e1 - t * e1.dot(t)).normalize();
        let e2 = t.cross(e1);
        let ring: Vec<u32> = (0..=sides)
            .map(|k| {
                let angle = k as f32 / sides as f32 * TAU;
                let n = e1 * angle.cos() + e2 * angle.sin();
                mesh.vertex(p + n * radius, n, [k as f32 / sides as f32, i as f32])
            })
            .collect();
        if let Some(last) = &previous {
            for k in 0..sides {
                mesh.quad([last[k], last[k + 1], ring[k + 1], ring[k]]);
            }
        }
        previous = Some(ring);
    }
}

/// A box centred at `centre` with half size `half`, turned by `rotation`.
pub fn cuboid(mesh: &mut MeshPart, centre: Vector3<f32>, half: Vector3<f32>, rotation: Quaternion<f32>) {
    let axes = [Vector3::unit_x(), Vector3::unit_y(), Vector3::unit_z()];
    for axis in 0..3 {
        for sign in [1.0f32, -1.0] {
            let n = axes[axis] * sign;
            // u × v = n
            let (u, v) = (axes[(axis + 1) % 3] * sign, axes[(axis + 2) % 3]);
            let (hu, hv) = (half[(axis + 1) % 3], half[(axis + 2) % 3]);
            let c = n * half[axis];
            let corner = |a: f32, b: f32| centre + rotation * (c + u * a * hu + v * b * hv);
            let normal = rotation * n;
            let corners = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                .map(|(a, b)| mesh.vertex(corner(a, b), normal, [a * 0.5 + 0.5, b * 0.5 + 0.5]));
            mesh.quad(corners);
        }
    }
}

/// Two perpendicular unit vectors across `axis` (normalized).
fn across(axis: Vector3<f32>) -> (Vector3<f32>, Vector3<f32>) {
    let helper = if axis.y.abs() < 0.9 { Vector3::unit_y() } else { Vector3::unit_x() };
    let e1 = axis.cross(helper).normalize();
    let e2 = axis.cross(e1);
    (e1, e2)
}

/// A closed cylinder from `a` to `b`.
pub fn cylinder(mesh: &mut MeshPart, a: Vector3<f32>, b: Vector3<f32>, radius: f32, sides: usize) {
    let d = b - a;
    if d.magnitude2() < 1e-10 {
        return;
    }
    let axis = d.normalize();
    let (e1, e2) = across(axis);
    let ring = |i: usize| {
        let angle = i as f32 / sides as f32 * TAU;
        e1 * angle.cos() + e2 * angle.sin()
    };
    for i in 0..sides {
        let (n0, n1) = (ring(i), ring(i + 1));
        let corners = [
            mesh.vertex(a + n0 * radius, n0, [0.0, 0.0]),
            mesh.vertex(a + n1 * radius, n1, [1.0, 0.0]),
            mesh.vertex(b + n1 * radius, n1, [1.0, 1.0]),
            mesh.vertex(b + n0 * radius, n0, [0.0, 1.0]),
        ];
        mesh.quad(corners);
    }
    for (centre, normal) in [(a, -axis), (b, axis)] {
        let middle = mesh.vertex(centre, normal, [0.5, 0.5]);
        for i in 0..sides {
            let p = mesh.vertex(centre + ring(i) * radius, normal, [0.0, 0.0]);
            let q = mesh.vertex(centre + ring(i + 1) * radius, normal, [1.0, 0.0]);
            mesh.triangles.push([middle, p, q]);
        }
    }
}

/// A ball made of `rings` × `segments` quads.
pub fn sphere(mesh: &mut MeshPart, centre: Vector3<f32>, radius: f32, rings: usize, segments: usize) {
    let point = |r: usize, s: usize| {
        let theta = r as f32 / rings as f32 * std::f32::consts::PI - FRAC_PI_2;
        let phi = s as f32 / segments as f32 * TAU;
        Vector3::new(theta.cos() * phi.cos(), theta.sin(), theta.cos() * phi.sin())
    };
    for r in 0..rings {
        for s in 0..segments {
            let ns = [point(r, s), point(r, s + 1), point(r + 1, s + 1), point(r + 1, s)];
            let corners = ns.map(|n| mesh.vertex(centre + n * radius, n, [0.0, 0.0]));
            mesh.quad(corners);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_device_has_a_port_and_a_look() {
        for kind in DeviceKind::ALL {
            assert!(!kind.ports().is_empty(), "{kind:?}");
            assert!(!kind.boxes().is_empty(), "{kind:?}");
            let parts = parts(kind);
            assert!(parts.iter().all(|p| !p.mesh.is_empty()), "{kind:?}");
        }
    }

    #[test]
    fn producers_consumers_and_batteries() {
        assert_eq!(DeviceKind::Solar.role(), Role::Producer);
        assert_eq!(DeviceKind::Lamp.role(), Role::Consumer);
        assert_eq!(DeviceKind::Battery.role(), Role::Storage);
        assert!(DeviceKind::Battery.capacity_wh() > 0.0);
        assert!(DeviceKind::ALL.iter().all(|k| k.watts() > 0.0));
    }

    #[test]
    fn a_solar_panel_faces_up_and_forward() {
        let n = solar_normal();
        assert!(n.y > 0.85 && n.z > 0.3);
    }

    #[test]
    fn ports_sit_on_or_next_to_their_device() {
        // A port is within a few centimetres of one of its boxes, so a
        // cable visibly plugs in.
        for kind in DeviceKind::ALL {
            for port in kind.ports() {
                let p = Vector3::from(*port);
                let near = kind.boxes().iter().any(|(c, h, r)| {
                    let local = r.conjugate() * (p - c);
                    (0..3).all(|i| local[i].abs() <= h[i] + 0.12)
                });
                assert!(near, "{kind:?} port {port:?}");
            }
        }
    }

    #[test]
    fn boxes_are_closed() {
        let mut m = MeshPart::default();
        cuboid(&mut m, Vector3::zero(), Vector3::new(1.0, 2.0, 3.0), Quaternion::new(1.0, 0.0, 0.0, 0.0));
        assert_eq!(m.triangles.len(), 12);
        // Each face's winding agrees with its normal.
        for t in &m.triangles {
            let [a, b, c] = t.map(|i| Vector3::from(m.positions[i as usize]));
            let n = Vector3::from(m.normals[t[0] as usize]);
            assert!((b - a).cross(c - a).dot(n) > 0.0);
        }
    }
}
