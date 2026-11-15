use cgmath::{InnerSpace, Vector3};
use serde::{Deserialize, Serialize};

use super::*;

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
    pub(super) fn renamed(&self, name: &str) -> Option<Self> {
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

    pub(super) fn shader_value(self) -> i32 {
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
