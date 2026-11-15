//! Particles: fire, smoke, embers, sparks, dust, wood chips, fireflies.
//!
//! A `ParticleEmitter` on an entity sends out particles as its
//! `ParticleEffect` says: from a shape around the entity, in a direction,
//! pulled by gravity and wind, slowed by drag, stirred by turbulence,
//! changing colour and size over their life. They are simulated on the CPU
//! (in play mode and in the editor, so effects can be tuned live) and drawn
//! as camera-facing quads in the transparent pass (`render`).
//!
//! An effect either glows (`emissive`, added on top of what is behind: fire,
//! sparks) or is lit like a surface (`lit`: smoke, dust), by the sun, the
//! moon, the sky and the scene's lights. Particles fade where they meet
//! geometry instead of cutting into it (`soft`).
//!
//! One-shot effects (a puff of dust when something is built, chips when a
//! tree is hit) are `spawn_burst`: an entity that removes itself when its
//! particles are gone.

mod render;

use std::cell::{Cell, RefCell};

use cgmath::{InnerSpace, Matrix4, Point3, SquareMatrix, Vector3, Vector4};
use glfw::{Glfw, WindowEvent};
use serde::{Deserialize, Serialize};

use crate::core::{
    entity::{
        component::{Component, Property, PropertyValue},
        Entity,
    },
    scene::{scene_file::ComponentData, Scene},
};

pub use render::ParticleRenderer;

/// Where particles start, around the emitter.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum EmitterShape {
    Point,
    /// Anywhere in a ball of this radius.
    Sphere(f32),
    /// Anywhere in a box of these half sizes.
    Box([f32; 3]),
    /// Anywhere on a flat disc of this radius (in the entity's xz plane).
    Disc(f32),
}

impl EmitterShape {
    pub const NAMES: [&'static str; 4] = ["Point", "Sphere", "Box", "Disc"];

    pub fn name(&self) -> &'static str {
        match self {
            EmitterShape::Point => "Point",
            EmitterShape::Sphere(_) => "Sphere",
            EmitterShape::Box(_) => "Box",
            EmitterShape::Disc(_) => "Disc",
        }
    }

    /// The shape called `name`, about as big as `self`.
    fn renamed(&self, name: &str) -> Option<Self> {
        let size = self.size();
        match name.to_ascii_lowercase().as_str() {
            "point" => Some(EmitterShape::Point),
            "sphere" => Some(EmitterShape::Sphere(size)),
            "box" => Some(EmitterShape::Box([size; 3])),
            "disc" => Some(EmitterShape::Disc(size)),
            _ => None,
        }
    }

    /// Its radius, or the box's largest half size.
    pub fn size(&self) -> f32 {
        match *self {
            EmitterShape::Point => 0.0,
            EmitterShape::Sphere(r) | EmitterShape::Disc(r) => r,
            EmitterShape::Box([x, y, z]) => x.max(y).max(z),
        }
    }

    /// A point in the shape, from three numbers in [0, 1).
    pub fn sample(&self, [a, b, c]: [f32; 3]) -> Vector3<f32> {
        match *self {
            EmitterShape::Point => Vector3::new(0.0, 0.0, 0.0),
            EmitterShape::Sphere(r) => {
                // Uniform in the ball: a direction, and the cube root for
                // the distance.
                let z = a * 2.0 - 1.0;
                let phi = b * std::f32::consts::TAU;
                let s = (1.0 - z * z).max(0.0).sqrt();
                Vector3::new(s * phi.cos(), z, s * phi.sin()) * (r * c.cbrt())
            }
            EmitterShape::Box([x, y, z]) => {
                Vector3::new((a * 2.0 - 1.0) * x, (b * 2.0 - 1.0) * y, (c * 2.0 - 1.0) * z)
            }
            EmitterShape::Disc(r) => {
                let phi = a * std::f32::consts::TAU;
                let d = r * b.sqrt();
                Vector3::new(d * phi.cos(), 0.0, d * phi.sin())
            }
        }
    }
}

/// How a particle looks.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticleLook {
    /// A round, soft glow.
    Soft,
    /// A ragged puff, different for every particle.
    Smoke,
    /// A streak along the way it flies, longer the faster.
    Spark,
}

impl ParticleLook {
    pub const ALL: [ParticleLook; 3] = [ParticleLook::Soft, ParticleLook::Smoke, ParticleLook::Spark];

    pub fn name(self) -> &'static str {
        match self {
            ParticleLook::Soft => "Soft",
            ParticleLook::Smoke => "Smoke",
            ParticleLook::Spark => "Spark",
        }
    }

    fn shader_value(self) -> i32 {
        match self {
            ParticleLook::Soft => 0,
            ParticleLook::Smoke => 1,
            ParticleLook::Spark => 2,
        }
    }
}

/// How particles go onto what is behind them.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticleBlend {
    /// Added on: light (fire, sparks). Order does not matter.
    Additive,
    /// Covering by their alpha (smoke, dust), drawn back to front.
    Alpha,
}

impl ParticleBlend {
    pub const ALL: [ParticleBlend; 2] = [ParticleBlend::Additive, ParticleBlend::Alpha];

    pub fn name(self) -> &'static str {
        match self {
            ParticleBlend::Additive => "Additive",
            ParticleBlend::Alpha => "Alpha",
        }
    }
}

/// Everything about an effect, as saved in a scene file. Colours are sRGB
/// with alpha, 0 to 1. Ranges are `[min, max]`; each particle gets a value
/// between them.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(default)]
pub struct ParticleEffect {
    pub enabled: bool,
    /// Particles a second.
    pub rate: f32,
    /// Particles all at once when it starts (and each time it loops).
    pub burst: u32,
    /// Starts again after `duration`; else it stops then.
    pub looping: bool,
    /// Seconds it emits for before it stops (or starts again).
    pub duration: f32,
    /// No more alive at once than this.
    pub max_particles: u32,
    /// Seconds each lives.
    pub lifetime: [f32; 2],
    pub shape: EmitterShape,
    /// Where they fly at the start, in the entity's space.
    pub direction: [f32; 3],
    /// Degrees they may fly off `direction`; 180 for every way.
    pub spread: f32,
    /// Metres a second at the start.
    pub speed: [f32; 2],
    /// Times earth's gravity pulling down; below 0 they rise (hot air).
    pub gravity: f32,
    /// Constant push, metres a second², in the world.
    pub wind: [f32; 3],
    /// How fast they lose speed (per second).
    pub drag: f32,
    /// How much they are stirred about, metres a second².
    pub turbulence: f32,
    /// Size across, metres, at the start and the end of their life.
    pub size: [f32; 2],
    /// Particles are this much bigger or smaller, at most (0.3: ±30%).
    pub size_jitter: f32,
    /// Turns a second, radians, at most (either way).
    pub spin: f32,
    /// Colour at the start, the middle and the end of their life.
    pub color_start: [f32; 4],
    pub color_middle: [f32; 4],
    pub color_end: [f32; 4],
    /// Times brighter than its colour: glowing (fire); 0 for not glowing.
    pub emissive: f32,
    /// Lit by the sun, moon, sky and lights (smoke, dust).
    pub lit: bool,
    pub look: ParticleLook,
    pub blend: ParticleBlend,
    /// Sparks: how much longer they are for their speed, seconds.
    pub stretch: f32,
    /// Metres over which they fade into geometry they meet.
    pub soft: f32,
    /// Particles stay where they were sent out when the entity moves;
    /// else they move with it.
    pub world_space: bool,
}

impl Default for ParticleEffect {
    fn default() -> Self {
        ParticlePreset::Fire.effect()
    }
}

/// Ready-made effects, as the editor offers them.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticlePreset {
    Fire,
    Smoke,
    Embers,
    Sparks,
    Dust,
    WoodChips,
    Fireflies,
    Mist,
}

impl ParticlePreset {
    pub const ALL: [ParticlePreset; 8] = [
        ParticlePreset::Fire,
        ParticlePreset::Smoke,
        ParticlePreset::Embers,
        ParticlePreset::Sparks,
        ParticlePreset::Dust,
        ParticlePreset::WoodChips,
        ParticlePreset::Fireflies,
        ParticlePreset::Mist,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ParticlePreset::Fire => "Fire",
            ParticlePreset::Smoke => "Smoke",
            ParticlePreset::Embers => "Embers",
            ParticlePreset::Sparks => "Sparks",
            ParticlePreset::Dust => "Dust",
            ParticlePreset::WoodChips => "Wood chips",
            ParticlePreset::Fireflies => "Fireflies",
            ParticlePreset::Mist => "Mist",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name().eq_ignore_ascii_case(name))
    }

    pub fn effect(self) -> ParticleEffect {
        let base = ParticleEffect {
            enabled: true,
            rate: 20.0,
            burst: 0,
            looping: true,
            duration: 5.0,
            max_particles: 400,
            lifetime: [1.0, 2.0],
            shape: EmitterShape::Point,
            direction: [0.0, 1.0, 0.0],
            spread: 20.0,
            speed: [0.5, 1.0],
            gravity: 0.0,
            wind: [0.0; 3],
            drag: 0.5,
            turbulence: 0.0,
            size: [0.3, 0.3],
            size_jitter: 0.2,
            spin: 0.5,
            color_start: [1.0; 4],
            color_middle: [1.0; 4],
            color_end: [1.0, 1.0, 1.0, 0.0],
            emissive: 0.0,
            lit: true,
            look: ParticleLook::Soft,
            blend: ParticleBlend::Alpha,
            stretch: 0.0,
            soft: 0.3,
            world_space: true,
        };
        match self {
            // Flames: quick, rising, yellow-white turning orange then a
            // dark red as they thin out.
            ParticlePreset::Fire => ParticleEffect {
                rate: 70.0,
                max_particles: 160,
                lifetime: [0.45, 0.9],
                shape: EmitterShape::Disc(0.28),
                spread: 12.0,
                speed: [0.4, 0.9],
                gravity: -0.25,
                drag: 1.2,
                turbulence: 1.4,
                size: [0.42, 0.12],
                size_jitter: 0.3,
                spin: 1.5,
                color_start: [1.0, 0.85, 0.45, 0.0],
                color_middle: [1.0, 0.45, 0.1, 0.8],
                color_end: [0.6, 0.08, 0.02, 0.0],
                emissive: 5.0,
                lit: false,
                look: ParticleLook::Smoke,
                blend: ParticleBlend::Additive,
                soft: 0.2,
                ..base
            },
            // Grey smoke rising, spreading and drifting off.
            ParticlePreset::Smoke => ParticleEffect {
                rate: 7.0,
                max_particles: 120,
                lifetime: [4.0, 7.0],
                shape: EmitterShape::Disc(0.2),
                spread: 10.0,
                speed: [0.6, 1.0],
                gravity: -0.05,
                wind: [0.25, 0.0, 0.1],
                drag: 0.4,
                turbulence: 0.35,
                size: [0.5, 2.6],
                size_jitter: 0.3,
                spin: 0.3,
                color_start: [0.35, 0.33, 0.32, 0.0],
                color_middle: [0.45, 0.44, 0.44, 0.45],
                color_end: [0.55, 0.55, 0.56, 0.0],
                look: ParticleLook::Smoke,
                blend: ParticleBlend::Alpha,
                soft: 0.6,
                ..base
            },
            // Glowing bits flying up out of a fire, swirling.
            ParticlePreset::Embers => ParticleEffect {
                rate: 9.0,
                max_particles: 60,
                lifetime: [1.2, 2.6],
                shape: EmitterShape::Disc(0.3),
                spread: 25.0,
                speed: [1.0, 2.4],
                gravity: -0.03,
                drag: 0.7,
                turbulence: 2.2,
                size: [0.035, 0.015],
                size_jitter: 0.3,
                color_start: [1.0, 0.75, 0.35, 1.0],
                color_middle: [1.0, 0.45, 0.12, 1.0],
                color_end: [0.8, 0.15, 0.05, 0.0],
                emissive: 8.0,
                lit: false,
                look: ParticleLook::Spark,
                blend: ParticleBlend::Additive,
                stretch: 0.04,
                soft: 0.0,
                ..base
            },
            // A shower of hot sparks, falling.
            ParticlePreset::Sparks => ParticleEffect {
                rate: 0.0,
                burst: 40,
                looping: false,
                duration: 0.1,
                max_particles: 80,
                lifetime: [0.3, 0.8],
                spread: 60.0,
                speed: [2.0, 5.0],
                gravity: 1.0,
                drag: 0.4,
                size: [0.03, 0.01],
                color_start: [1.0, 0.95, 0.7, 1.0],
                color_middle: [1.0, 0.6, 0.2, 1.0],
                color_end: [0.9, 0.25, 0.05, 0.0],
                emissive: 12.0,
                lit: false,
                look: ParticleLook::Spark,
                blend: ParticleBlend::Additive,
                stretch: 0.03,
                soft: 0.0,
                ..base
            },
            // A puff of dust that settles: building, landing.
            ParticlePreset::Dust => ParticleEffect {
                rate: 0.0,
                burst: 24,
                looping: false,
                duration: 0.1,
                max_particles: 40,
                lifetime: [1.2, 2.2],
                shape: EmitterShape::Box([1.2, 0.3, 1.2]),
                spread: 75.0,
                speed: [0.6, 1.6],
                gravity: 0.02,
                drag: 1.8,
                turbulence: 0.3,
                size: [0.5, 1.4],
                size_jitter: 0.35,
                color_start: [0.62, 0.56, 0.48, 0.55],
                color_middle: [0.66, 0.6, 0.52, 0.35],
                color_end: [0.7, 0.65, 0.58, 0.0],
                look: ParticleLook::Smoke,
                soft: 0.4,
                ..base
            },
            // Chips flying off a trunk an axe hits, tumbling down.
            ParticlePreset::WoodChips => ParticleEffect {
                rate: 0.0,
                burst: 26,
                looping: false,
                duration: 0.1,
                max_particles: 40,
                lifetime: [0.8, 1.6],
                shape: EmitterShape::Sphere(0.15),
                spread: 75.0,
                speed: [1.5, 3.5],
                gravity: 1.0,
                drag: 0.6,
                size: [0.07, 0.06],
                size_jitter: 0.4,
                spin: 9.0,
                color_start: [0.78, 0.62, 0.4, 1.0],
                color_middle: [0.72, 0.56, 0.36, 1.0],
                color_end: [0.6, 0.46, 0.3, 0.0],
                look: ParticleLook::Smoke,
                soft: 0.0,
                ..base
            },
            // Little green-yellow lights drifting about over a wide area.
            ParticlePreset::Fireflies => ParticleEffect {
                rate: 7.0,
                max_particles: 80,
                lifetime: [5.0, 9.0],
                shape: EmitterShape::Box([8.0, 1.2, 8.0]),
                spread: 180.0,
                speed: [0.1, 0.3],
                drag: 0.8,
                turbulence: 0.6,
                size: [0.13, 0.13],
                size_jitter: 0.2,
                spin: 0.0,
                color_start: [0.75, 1.0, 0.35, 0.0],
                color_middle: [0.8, 1.0, 0.4, 1.0],
                color_end: [0.7, 1.0, 0.3, 0.0],
                emissive: 7.0,
                lit: false,
                blend: ParticleBlend::Additive,
                soft: 0.0,
                ..base
            },
            // Low mist creeping over the ground or water.
            ParticlePreset::Mist => ParticleEffect {
                rate: 3.0,
                max_particles: 80,
                lifetime: [10.0, 16.0],
                shape: EmitterShape::Box([10.0, 0.3, 10.0]),
                spread: 180.0,
                speed: [0.05, 0.15],
                wind: [0.05, 0.0, 0.02],
                drag: 0.2,
                turbulence: 0.05,
                size: [3.0, 5.0],
                size_jitter: 0.3,
                spin: 0.05,
                color_start: [0.8, 0.82, 0.85, 0.0],
                color_middle: [0.8, 0.82, 0.85, 0.22],
                color_end: [0.8, 0.82, 0.85, 0.0],
                look: ParticleLook::Smoke,
                soft: 1.5,
                ..base
            },
        }
    }
}

impl ParticleEffect {
    pub const RATE_RANGE: (f32, f32) = (0.0, 2000.0);
    pub const MAX_PARTICLES: u32 = 10_000;

    /// Every value in its range, ranges in order (for hand-edited files).
    pub fn sanitized(mut self) -> Self {
        let finite = |v: f32, fallback: f32| if v.is_finite() { v } else { fallback };
        let range = |[a, b]: [f32; 2], min: f32, max: f32| {
            let (a, b) = (finite(a, min).clamp(min, max), finite(b, min).clamp(min, max));
            [a.min(b), a.max(b)]
        };
        self.rate = finite(self.rate, 0.0).clamp(Self::RATE_RANGE.0, Self::RATE_RANGE.1);
        self.burst = self.burst.min(Self::MAX_PARTICLES);
        self.duration = finite(self.duration, 1.0).clamp(0.01, 3600.0);
        self.max_particles = self.max_particles.clamp(1, Self::MAX_PARTICLES);
        self.lifetime = range(self.lifetime, 0.01, 600.0);
        self.speed = range(self.speed, 0.0, 200.0);
        self.size = self.size.map(|s| finite(s, 0.1).clamp(0.0, 100.0));
        self.size_jitter = finite(self.size_jitter, 0.0).clamp(0.0, 1.0);
        self.spread = finite(self.spread, 0.0).clamp(0.0, 180.0);
        self.drag = finite(self.drag, 0.0).clamp(0.0, 50.0);
        self.turbulence = finite(self.turbulence, 0.0).clamp(0.0, 100.0);
        self.gravity = finite(self.gravity, 0.0).clamp(-10.0, 10.0);
        self.spin = finite(self.spin, 0.0).clamp(0.0, 50.0);
        self.emissive = finite(self.emissive, 0.0).clamp(0.0, 100.0);
        self.stretch = finite(self.stretch, 0.0).clamp(0.0, 1.0);
        self.soft = finite(self.soft, 0.0).clamp(0.0, 10.0);
        for color in [&mut self.color_start, &mut self.color_middle, &mut self.color_end] {
            *color = color.map(|c| finite(c, 1.0).clamp(0.0, 1.0));
        }
        self.shape = match self.shape {
            EmitterShape::Sphere(r) => EmitterShape::Sphere(finite(r, 0.5).clamp(0.0, 1000.0)),
            EmitterShape::Disc(r) => EmitterShape::Disc(finite(r, 0.5).clamp(0.0, 1000.0)),
            EmitterShape::Box(h) => EmitterShape::Box(h.map(|v| finite(v, 0.5).clamp(0.0, 1000.0))),
            EmitterShape::Point => EmitterShape::Point,
        };
        let d = Vector3::from(self.direction.map(|v| finite(v, 0.0)));
        self.direction = if d.magnitude2() > 1e-12 {
            d.normalize().into()
        } else {
            [0.0, 1.0, 0.0]
        };
        self.wind = self.wind.map(|v| finite(v, 0.0).clamp(-100.0, 100.0));
        self
    }

    /// The colour (sRGB with alpha) at `t` of a particle's life (0..1).
    pub fn color_at(&self, t: f32) -> [f32; 4] {
        let t = t.clamp(0.0, 1.0);
        let (a, b, f) = if t < 0.5 {
            (self.color_start, self.color_middle, t * 2.0)
        } else {
            (self.color_middle, self.color_end, t * 2.0 - 1.0)
        };
        std::array::from_fn(|i| a[i] + (b[i] - a[i]) * f)
    }

    /// The size at `t` of a particle's life (0..1).
    pub fn size_at(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        self.size[0] + (self.size[1] - self.size[0]) * t
    }
}

/// One particle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Particle {
    /// In the world, or the entity's space without `world_space`.
    pub position: Vector3<f32>,
    pub velocity: Vector3<f32>,
    pub age: f32,
    pub lifetime: f32,
    /// Times the effect's size.
    pub scale: f32,
    pub rotation: f32,
    pub spin: f32,
    /// 0..1, different for each: picks its puff shape and stirring.
    pub seed: f32,
}

/// A small fast random number generator (xorshift).
#[derive(Clone, Debug)]
struct Random(u32);

impl Random {
    fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    fn between(&mut self, [a, b]: [f32; 2]) -> f32 {
        a + (b - a) * self.next()
    }
}

/// An effect's particles and where it is in its cycle: the simulation,
/// without anything GPU, for tests and the server too.
#[derive(Clone, Debug)]
pub struct ParticleSystem {
    pub effect: ParticleEffect,
    pub particles: Vec<Particle>,
    /// Seconds into the current cycle.
    time: f32,
    /// Particles owed but not sent out yet (a fraction of one).
    owed: f32,
    burst_due: bool,
    /// Ran its duration without looping: sends out nothing more.
    stopped: bool,
    clock: f32,
    random: Random,
}

impl ParticleSystem {
    pub fn new(effect: ParticleEffect, seed: u32) -> Self {
        Self {
            effect: effect.sanitized(),
            particles: Vec::new(),
            time: 0.0,
            owed: 0.0,
            burst_due: true,
            stopped: false,
            clock: 0.0,
            random: Random(seed.max(1)),
        }
    }

    /// Start the effect over: a new burst, no particles left.
    pub fn restart(&mut self) {
        self.particles.clear();
        self.time = 0.0;
        self.owed = 0.0;
        self.burst_due = true;
        self.stopped = false;
    }

    /// Done: stopped and every particle gone.
    pub fn finished(&self) -> bool {
        self.stopped && self.particles.is_empty()
    }

    /// Move it on by `dt` seconds; new particles start from `transform`
    /// (the emitter's world transform).
    pub fn step(&mut self, dt: f32, transform: &Matrix4<f32>) {
        let dt = dt.clamp(0.0, 0.1);
        self.clock += dt;
        let effect = self.effect;

        // Age and move what is there.
        let gravity = Vector3::new(0.0, -9.81 * effect.gravity, 0.0);
        let wind = Vector3::from(effect.wind);
        let keep = (-effect.drag * dt).exp();
        let clock = self.clock;
        self.particles.retain_mut(|p| {
            p.age += dt;
            if p.age >= p.lifetime {
                return false;
            }
            let mut push = gravity + wind;
            if effect.turbulence > 0.0 {
                push += stir(p.position, clock, p.seed) * effect.turbulence;
            }
            p.velocity = (p.velocity + push * dt) * keep;
            p.position += p.velocity * dt;
            p.rotation += p.spin * dt;
            true
        });

        if !effect.enabled || self.stopped {
            return;
        }
        let mut count = 0usize;
        if self.burst_due {
            self.burst_due = false;
            count += effect.burst as usize;
        }
        self.time += dt;
        self.owed += effect.rate * dt;
        let whole = self.owed.floor();
        self.owed -= whole;
        count += whole as usize;
        if self.time >= effect.duration {
            if effect.looping {
                self.time -= effect.duration;
                self.burst_due = effect.burst > 0;
            } else {
                self.stopped = true;
            }
        }
        let room = (effect.max_particles as usize).saturating_sub(self.particles.len());
        for _ in 0..count.min(room) {
            let p = self.spawn(transform);
            self.particles.push(p);
        }
    }

    fn spawn(&mut self, transform: &Matrix4<f32>) -> Particle {
        let e = self.effect;
        let r = &mut self.random;
        let local = e.shape.sample([r.next(), r.next(), r.next()]);
        let direction = cone(Vector3::from(e.direction), e.spread.to_radians(), r.next(), r.next());
        let speed = r.between(e.speed);
        let (position, velocity) = if e.world_space {
            let p = transform * Vector4::new(local.x, local.y, local.z, 1.0);
            let d = (transform * direction.extend(0.0)).truncate();
            let d = if d.magnitude2() > 1e-12 { d.normalize() } else { direction };
            (p.truncate() / p.w, d * speed)
        } else {
            (local, direction * speed)
        };
        let jitter = e.size_jitter;
        Particle {
            position,
            velocity,
            age: 0.0,
            lifetime: r.between(e.lifetime),
            scale: 1.0 + (r.next() * 2.0 - 1.0) * jitter,
            rotation: r.next() * std::f32::consts::TAU,
            spin: (r.next() * 2.0 - 1.0) * e.spin,
            seed: r.next(),
        }
    }
}

/// A direction within `angle` (radians) of `axis`, from two numbers in
/// [0, 1): even over the cap of the sphere.
pub fn cone(axis: Vector3<f32>, angle: f32, a: f32, b: f32) -> Vector3<f32> {
    let axis = if axis.magnitude2() > 1e-12 {
        axis.normalize()
    } else {
        Vector3::unit_y()
    };
    let cos = 1.0 - a * (1.0 - angle.cos());
    let sin = (1.0 - cos * cos).max(0.0).sqrt();
    let phi = b * std::f32::consts::TAU;
    let helper = if axis.x.abs() < 0.9 {
        Vector3::unit_x()
    } else {
        Vector3::unit_y()
    };
    let u = axis.cross(helper).normalize();
    let v = axis.cross(u);
    axis * cos + (u * phi.cos() + v * phi.sin()) * sin
}

/// A smooth swirl to stir particles with, about -1..1 on each axis.
fn stir(p: Vector3<f32>, t: f32, seed: f32) -> Vector3<f32> {
    let s = seed * 17.0;
    Vector3::new(
        (p.y * 1.7 + t * 1.3 + s).sin() + (p.z * 0.9 - t * 0.7).sin() * 0.5,
        (p.z * 1.9 + t * 1.1 + s * 1.3).sin() * 0.6,
        (p.x * 1.5 - t * 1.7 + s * 0.7).sin() + (p.y * 1.1 + t * 0.9).sin() * 0.5,
    ) * 0.67
}

/// An effect on an entity. It runs in play mode and while the scene is
/// edited, so what it looks like can be tuned live.
pub struct ParticleEmitter {
    system: ParticleSystem,
    /// The entity's world transform when it was last drawn: where new
    /// particles start. None until it is drawn the first time.
    transform: Cell<Option<Matrix4<f32>>>,
    /// Removes its entity once its particles are gone (one-shot effects).
    pub despawn_when_done: bool,
    /// What `render` draws with: the particles sorted, packed.
    scratch: RefCell<Vec<[f32; 16]>>,
}

impl ParticleEmitter {
    pub fn new(effect: ParticleEffect) -> Self {
        // Different emitters of the same effect should not look the same.
        static SEED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x9E37_79B9);
        let seed = SEED.fetch_add(0x6D2B_79F5, std::sync::atomic::Ordering::Relaxed);
        Self {
            system: ParticleSystem::new(effect, seed),
            transform: Cell::new(None),
            despawn_when_done: false,
            scratch: RefCell::new(Vec::new()),
        }
    }

    pub fn effect(&self) -> &ParticleEffect {
        &self.system.effect
    }

    pub fn set_effect(&mut self, effect: ParticleEffect) {
        self.system.effect = effect.sanitized();
    }

    pub fn system(&self) -> &ParticleSystem {
        &self.system
    }

    /// Start over (a burst again).
    pub fn restart(&mut self) {
        self.system.restart();
    }

    fn step(&mut self, scene: &mut Scene, entity: &Entity, delta_time: f64) {
        // Not drawn yet: where it is in the world is not known.
        let Some(transform) = self.transform.get() else {
            return;
        };
        self.system.step(delta_time as f32, &transform);
        if self.despawn_when_done && self.system.finished() {
            let id = entity.id;
            self.despawn_when_done = false;
            scene.defer(move |scene| {
                scene.remove_entity(&id);
            });
        }
    }
}

/// A one-shot effect (a burst, not looping) at `position`: an entity that
/// removes itself when its particles are gone. Added once every entity is
/// updated.
pub fn spawn_burst(scene: &mut Scene, effect: ParticleEffect, position: Point3<f32>) {
    let mut emitter = ParticleEmitter::new(ParticleEffect {
        looping: false,
        ..effect
    });
    emitter.despawn_when_done = true;
    let mut entity = Entity::new("particles");
    entity.set_transient(true);
    entity.set_local_position(position);
    // Known at once, so it can start before it is first drawn.
    emitter
        .transform
        .set(Some(Matrix4::from_translation(Vector3::new(position.x, position.y, position.z))));
    entity.add_component(emitter);
    scene.defer(move |scene| scene.add_entity(entity));
}

/// A float field of an effect, as the inspector shows it.
type FloatField = (&'static str, f32, f32, fn(&ParticleEffect) -> f32, fn(&mut ParticleEffect, f32));

fn float_fields() -> Vec<FloatField> {
    fn shape_size(e: &ParticleEffect) -> f32 {
        e.shape.size()
    }
    fn set_shape_size(e: &mut ParticleEffect, v: f32) {
        e.shape = match e.shape {
            EmitterShape::Point => EmitterShape::Point,
            EmitterShape::Sphere(_) => EmitterShape::Sphere(v),
            EmitterShape::Disc(_) => EmitterShape::Disc(v),
            EmitterShape::Box(_) => EmitterShape::Box([v; 3]),
        }
    }
    vec![
        ("rate", 0.0, 500.0, |e| e.rate, |e, v| e.rate = v),
        ("burst", 0.0, 500.0, |e| e.burst as f32, |e, v| e.burst = v.max(0.0).round() as u32),
        ("duration (s)", 0.05, 60.0, |e| e.duration, |e, v| e.duration = v),
        ("max particles", 1.0, 5000.0, |e| e.max_particles as f32, |e, v| e.max_particles = v.max(1.0).round() as u32),
        ("lifetime min", 0.05, 30.0, |e| e.lifetime[0], |e, v| e.lifetime[0] = v),
        ("lifetime max", 0.05, 30.0, |e| e.lifetime[1], |e, v| e.lifetime[1] = v),
        ("shape size", 0.0, 20.0, shape_size, set_shape_size),
        ("direction x", -1.0, 1.0, |e| e.direction[0], |e, v| e.direction[0] = v),
        ("direction y", -1.0, 1.0, |e| e.direction[1], |e, v| e.direction[1] = v),
        ("direction z", -1.0, 1.0, |e| e.direction[2], |e, v| e.direction[2] = v),
        ("spread", 0.0, 180.0, |e| e.spread, |e, v| e.spread = v),
        ("speed min", 0.0, 20.0, |e| e.speed[0], |e, v| e.speed[0] = v),
        ("speed max", 0.0, 20.0, |e| e.speed[1], |e, v| e.speed[1] = v),
        ("gravity", -2.0, 2.0, |e| e.gravity, |e, v| e.gravity = v),
        ("wind x", -5.0, 5.0, |e| e.wind[0], |e, v| e.wind[0] = v),
        ("wind z", -5.0, 5.0, |e| e.wind[2], |e, v| e.wind[2] = v),
        ("drag", 0.0, 10.0, |e| e.drag, |e, v| e.drag = v),
        ("turbulence", 0.0, 10.0, |e| e.turbulence, |e, v| e.turbulence = v),
        ("size start", 0.0, 10.0, |e| e.size[0], |e, v| e.size[0] = v),
        ("size end", 0.0, 10.0, |e| e.size[1], |e, v| e.size[1] = v),
        ("size jitter", 0.0, 1.0, |e| e.size_jitter, |e, v| e.size_jitter = v),
        ("spin", 0.0, 20.0, |e| e.spin, |e, v| e.spin = v),
        ("start r", 0.0, 1.0, |e| e.color_start[0], |e, v| e.color_start[0] = v),
        ("start g", 0.0, 1.0, |e| e.color_start[1], |e, v| e.color_start[1] = v),
        ("start b", 0.0, 1.0, |e| e.color_start[2], |e, v| e.color_start[2] = v),
        ("start a", 0.0, 1.0, |e| e.color_start[3], |e, v| e.color_start[3] = v),
        ("middle r", 0.0, 1.0, |e| e.color_middle[0], |e, v| e.color_middle[0] = v),
        ("middle g", 0.0, 1.0, |e| e.color_middle[1], |e, v| e.color_middle[1] = v),
        ("middle b", 0.0, 1.0, |e| e.color_middle[2], |e, v| e.color_middle[2] = v),
        ("middle a", 0.0, 1.0, |e| e.color_middle[3], |e, v| e.color_middle[3] = v),
        ("end r", 0.0, 1.0, |e| e.color_end[0], |e, v| e.color_end[0] = v),
        ("end g", 0.0, 1.0, |e| e.color_end[1], |e, v| e.color_end[1] = v),
        ("end b", 0.0, 1.0, |e| e.color_end[2], |e, v| e.color_end[2] = v),
        ("end a", 0.0, 1.0, |e| e.color_end[3], |e, v| e.color_end[3] = v),
        ("emissive", 0.0, 30.0, |e| e.emissive, |e, v| e.emissive = v),
        ("stretch", 0.0, 0.2, |e| e.stretch, |e, v| e.stretch = v),
        ("soft", 0.0, 3.0, |e| e.soft, |e, v| e.soft = v),
    ]
}

impl Component for ParticleEmitter {
    fn update(&mut self, scene: &mut Scene, entity: &mut Entity, delta_time: f64) {
        self.step(scene, entity, delta_time);
    }

    fn update_in_editor(&mut self, scene: &mut Scene, entity: &mut Entity, delta_time: f64) {
        // A one-shot effect plays again and again while it is edited.
        if self.system.finished() && !self.despawn_when_done {
            self.system.restart();
        }
        self.step(scene, entity, delta_time);
    }

    fn render_transparent(
        &self,
        scene: &Scene,
        _: &Entity,
        view_projection: &Matrix4<f32>,
        parent_transform: &Matrix4<f32>,
    ) {
        self.transform.set(Some(*parent_transform));
        if self.system.particles.is_empty() {
            return;
        }
        let local = if self.system.effect.world_space {
            Matrix4::identity()
        } else {
            *parent_transform
        };
        render::draw(scene, &self.system, &local, view_projection, &mut self.scratch.borrow_mut());
    }

    fn handle_event(&mut self, _: &mut Glfw, _: &mut glfw::Window, _: &WindowEvent) {}

    fn display_name(&self) -> String {
        "Particles".into()
    }

    fn to_data(&self) -> Option<ComponentData> {
        // One-shot effects made by the game are not part of the scene.
        (!self.despawn_when_done).then(|| ComponentData::Particles {
            effect: self.system.effect,
        })
    }

    fn properties(&self) -> Vec<Property> {
        let e = &self.system.effect;
        let mut properties = vec![
            Property::choice(
                "preset",
                // The preset it is, until something is changed.
                ParticlePreset::ALL
                    .iter()
                    .find(|p| p.effect() == *e)
                    .map_or("Custom", |p| p.name())
                    .into(),
                std::iter::once("Custom".to_string())
                    .chain(ParticlePreset::ALL.iter().map(|p| p.name().to_string()))
                    .collect(),
            ),
            Property::bool("enabled", e.enabled),
            Property::bool("looping", e.looping),
            Property::choice(
                "shape",
                e.shape.name().into(),
                EmitterShape::NAMES.iter().map(|s| s.to_string()).collect(),
            ),
            Property::choice(
                "look",
                e.look.name().into(),
                ParticleLook::ALL.iter().map(|l| l.name().to_string()).collect(),
            ),
            Property::choice(
                "blend",
                e.blend.name().into(),
                ParticleBlend::ALL.iter().map(|b| b.name().to_string()).collect(),
            ),
            Property::bool("lit", e.lit),
            Property::bool("world space", e.world_space),
        ];
        for (name, min, max, get, _) in float_fields() {
            properties.push(Property::ranged(name, get(e), min, max));
        }
        properties.push(Property::int("alive", self.system.particles.len() as i64).read_only());
        properties
    }

    fn set_property(&mut self, name: &str, value: PropertyValue) {
        let mut e = self.system.effect;
        match name {
            "preset" => match value.as_text().and_then(ParticlePreset::from_name) {
                Some(preset) => {
                    e = preset.effect();
                    self.system.restart();
                }
                None => return,
            },
            "enabled" => e.enabled = value.as_bool().unwrap_or(e.enabled),
            "looping" => e.looping = value.as_bool().unwrap_or(e.looping),
            "lit" => e.lit = value.as_bool().unwrap_or(e.lit),
            "world space" => e.world_space = value.as_bool().unwrap_or(e.world_space),
            "shape" => {
                if let Some(shape) = value.as_text().and_then(|n| e.shape.renamed(n)) {
                    e.shape = shape;
                }
            }
            "look" => {
                if let Some(look) = value
                    .as_text()
                    .and_then(|n| ParticleLook::ALL.into_iter().find(|l| l.name().eq_ignore_ascii_case(n)))
                {
                    e.look = look;
                }
            }
            "blend" => {
                if let Some(blend) = value
                    .as_text()
                    .and_then(|n| ParticleBlend::ALL.into_iter().find(|b| b.name().eq_ignore_ascii_case(n)))
                {
                    e.blend = blend;
                }
            }
            _ => {
                let Some(v) = value.as_f32() else {
                    return;
                };
                match float_fields().into_iter().find(|(n, ..)| *n == name) {
                    Some((_, _, _, _, set)) => set(&mut e, v),
                    None => return,
                }
            }
        }
        let was_looping = self.system.effect.looping;
        self.set_effect(e);
        // Switched to looping after it stopped: start again.
        if self.system.effect.looping && !was_looping {
            self.system.restart();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quiet(effect: ParticleEffect) -> ParticleSystem {
        ParticleSystem::new(effect, 12345)
    }

    fn at(x: f32, y: f32, z: f32) -> Matrix4<f32> {
        Matrix4::from_translation(Vector3::new(x, y, z))
    }

    #[test]
    fn particles_come_at_the_rate_asked_for() {
        let mut s = quiet(ParticleEffect {
            rate: 10.0,
            lifetime: [100.0, 100.0],
            max_particles: 1000,
            ..ParticlePreset::Smoke.effect()
        });
        for _ in 0..100 {
            s.step(0.01, &at(0.0, 0.0, 0.0));
        }
        // One second: ten, give or take the one owed.
        assert!((9..=10).contains(&s.particles.len()), "{}", s.particles.len());
    }

    #[test]
    fn a_burst_comes_at_once_and_a_one_shot_finishes() {
        let mut s = quiet(ParticlePreset::Sparks.effect());
        s.step(0.016, &at(0.0, 0.0, 0.0));
        assert_eq!(s.particles.len(), 40);
        assert!(!s.finished());
        for _ in 0..200 {
            s.step(0.016, &at(0.0, 0.0, 0.0));
        }
        assert!(s.finished());
        s.restart();
        s.step(0.016, &at(0.0, 0.0, 0.0));
        assert_eq!(s.particles.len(), 40);
    }

    #[test]
    fn never_more_than_the_maximum() {
        let mut s = quiet(ParticleEffect {
            rate: 1000.0,
            burst: 500,
            max_particles: 50,
            lifetime: [10.0, 10.0],
            ..ParticlePreset::Fire.effect()
        });
        for _ in 0..10 {
            s.step(0.05, &at(0.0, 0.0, 0.0));
        }
        assert_eq!(s.particles.len(), 50);
    }

    #[test]
    fn particles_die_at_the_end_of_their_life() {
        let mut s = quiet(ParticleEffect {
            rate: 0.0,
            burst: 5,
            lifetime: [0.5, 0.5],
            ..ParticlePreset::Embers.effect()
        });
        s.step(0.01, &at(0.0, 0.0, 0.0));
        assert_eq!(s.particles.len(), 5);
        for _ in 0..60 {
            s.step(0.01, &at(0.0, 0.0, 0.0));
        }
        assert!(s.particles.is_empty());
    }

    #[test]
    fn gravity_pulls_them_down_and_negative_gravity_lifts_them() {
        for (gravity, down) in [(1.0, true), (-1.0, false)] {
            let mut s = quiet(ParticleEffect {
                rate: 0.0,
                burst: 1,
                speed: [0.0, 0.0],
                gravity,
                drag: 0.0,
                turbulence: 0.0,
                wind: [0.0; 3],
                lifetime: [5.0, 5.0],
                shape: EmitterShape::Point,
                ..ParticlePreset::Dust.effect()
            });
            for _ in 0..50 {
                s.step(0.01, &at(0.0, 10.0, 0.0));
            }
            let y = s.particles[0].position.y;
            // Half a second: about 1.2 m.
            assert!((y - 10.0).abs() > 1.0, "{y}");
            assert_eq!(y < 10.0, down);
        }
    }

    #[test]
    fn they_start_inside_the_shape_around_the_emitter() {
        for shape in [
            EmitterShape::Sphere(2.0),
            EmitterShape::Box([1.0, 0.5, 3.0]),
            EmitterShape::Disc(1.5),
        ] {
            let mut s = quiet(ParticleEffect {
                rate: 0.0,
                burst: 200,
                max_particles: 200,
                speed: [0.0, 0.0],
                shape,
                ..ParticlePreset::Dust.effect()
            });
            s.step(0.0, &at(10.0, 20.0, 30.0));
            for p in &s.particles {
                let d = p.position - Vector3::new(10.0, 20.0, 30.0);
                let inside = match shape {
                    EmitterShape::Sphere(r) => d.magnitude() <= r + 1e-4,
                    EmitterShape::Box([x, y, z]) => d.x.abs() <= x && d.y.abs() <= y && d.z.abs() <= z,
                    EmitterShape::Disc(r) => d.y.abs() < 1e-4 && (d.x * d.x + d.z * d.z).sqrt() <= r + 1e-4,
                    EmitterShape::Point => true,
                };
                assert!(inside, "{shape:?}: {d:?}");
            }
        }
    }

    #[test]
    fn they_fly_within_the_spread_of_the_direction() {
        for _ in 0..100 {
            let a = (0.37f32 * 7.0).fract();
            let d = cone(Vector3::unit_y(), 20f32.to_radians(), a, 0.6);
            assert!(d.dot(Vector3::unit_y()) >= 20f32.to_radians().cos() - 1e-5);
            assert!((d.magnitude() - 1.0).abs() < 1e-5);
        }
        let mut s = quiet(ParticleEffect {
            rate: 0.0,
            burst: 100,
            spread: 15.0,
            speed: [1.0, 1.0],
            ..ParticlePreset::Embers.effect()
        });
        // The entity turned upside down: they fly down.
        let flipped = Matrix4::from_angle_x(cgmath::Deg(180.0));
        s.step(0.0, &flipped);
        for p in &s.particles {
            assert!(p.velocity.normalize().y < -15f32.to_radians().cos() + 1e-4);
        }
    }

    #[test]
    fn colour_and_size_change_over_their_life() {
        let e = ParticlePreset::Fire.effect();
        let near = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-5);
        assert!(near(e.color_at(0.0), e.color_start));
        assert!(near(e.color_at(0.5), e.color_middle));
        assert!(near(e.color_at(1.0), e.color_end));
        let quarter = e.color_at(0.25);
        assert!((quarter[3] - (e.color_start[3] + e.color_middle[3]) / 2.0).abs() < 1e-6);
        assert!((e.size_at(0.5) - (e.size[0] + e.size[1]) / 2.0).abs() < 1e-6);
    }

    #[test]
    fn hand_edited_effects_are_put_in_range() {
        let e = ParticleEffect {
            rate: -5.0,
            lifetime: [3.0, f32::NAN],
            speed: [5.0, 1.0],
            spread: 400.0,
            max_particles: 0,
            direction: [0.0, 0.0, 0.0],
            color_start: [2.0, -1.0, 0.5, 1.0],
            ..ParticlePreset::Fire.effect()
        }
        .sanitized();
        assert_eq!(e.rate, 0.0);
        assert!(e.lifetime[0] <= e.lifetime[1]);
        assert_eq!(e.speed, [1.0, 5.0]);
        assert_eq!(e.spread, 180.0);
        assert_eq!(e.max_particles, 1);
        assert_eq!(e.direction, [0.0, 1.0, 0.0]);
        assert_eq!(e.color_start, [1.0, 0.0, 0.5, 1.0]);
    }

    #[test]
    fn every_preset_is_in_range_and_has_a_name() {
        for preset in ParticlePreset::ALL {
            let e = preset.effect();
            assert_eq!(e.sanitized(), e, "{}", preset.name());
            assert_eq!(ParticlePreset::from_name(preset.name()), Some(preset));
        }
    }

    #[test]
    fn effects_round_trip_through_ron_and_may_leave_out_fields() {
        let e = ParticlePreset::Smoke.effect();
        let text = ron::to_string(&e).unwrap();
        assert_eq!(ron::from_str::<ParticleEffect>(&text).unwrap(), e);
        let short: ParticleEffect = ron::from_str("(rate: 3.0, shape: Sphere(1.0))").unwrap();
        assert_eq!(short.rate, 3.0);
        assert_eq!(short.shape, EmitterShape::Sphere(1.0));
        assert_eq!(short.look, ParticleEffect::default().look);
    }

    #[test]
    fn the_inspector_reads_and_writes_every_field() {
        let mut emitter = ParticleEmitter::new(ParticlePreset::Fire.effect());
        emitter.set_property("rate", PropertyValue::Float(12.0));
        emitter.set_property("end a", PropertyValue::Float(0.5));
        emitter.set_property("shape", PropertyValue::Text("box".into()));
        emitter.set_property("blend", PropertyValue::Text("Alpha".into()));
        let e = *emitter.effect();
        assert_eq!(e.rate, 12.0);
        assert_eq!(e.color_end[3], 0.5);
        assert!(matches!(e.shape, EmitterShape::Box(_)));
        assert_eq!(e.blend, ParticleBlend::Alpha);
        emitter.set_property("preset", PropertyValue::Text("Smoke".into()));
        assert_eq!(*emitter.effect(), ParticlePreset::Smoke.effect());
        let shown = |emitter: &ParticleEmitter| {
            emitter.properties().into_iter().find(|p| p.name == "preset").unwrap().value
        };
        assert_eq!(shown(&emitter), PropertyValue::Text("Smoke".into()));
        // Every float it shows it takes back unchanged.
        for p in emitter.properties() {
            if p.read_only {
                continue;
            }
            let before = *emitter.effect();
            emitter.set_property(&p.name, p.value.clone());
            assert_eq!(*emitter.effect(), before, "{}", p.name);
        }
    }

    #[test]
    fn it_saves_as_its_effect_but_one_shots_do_not() {
        let mut emitter = ParticleEmitter::new(ParticlePreset::Embers.effect());
        assert_eq!(
            emitter.to_data(),
            Some(ComponentData::Particles {
                effect: ParticlePreset::Embers.effect()
            })
        );
        emitter.despawn_when_done = true;
        assert_eq!(emitter.to_data(), None);
    }

    #[test]
    fn a_moving_emitter_leaves_world_space_particles_behind() {
        let mut s = quiet(ParticleEffect {
            rate: 0.0,
            burst: 1,
            speed: [0.0, 0.0],
            gravity: 0.0,
            turbulence: 0.0,
            wind: [0.0; 3],
            shape: EmitterShape::Point,
            ..ParticlePreset::Smoke.effect()
        });
        s.step(0.01, &at(1.0, 2.0, 3.0));
        s.step(0.01, &at(50.0, 0.0, 0.0));
        assert!((s.particles[0].position - Vector3::new(1.0, 2.0, 3.0)).magnitude() < 1e-4);
    }
}
