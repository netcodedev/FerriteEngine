//! Simulating an effect's particles on the CPU.

use cgmath::{InnerSpace, Matrix4, Vector3, Vector4};

use super::*;

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
