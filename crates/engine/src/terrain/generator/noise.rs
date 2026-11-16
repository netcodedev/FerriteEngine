//! Noise fields of the world: simplex fractals, each from its own seed.

use libnoise::{Generator, Simplex, Source};

/// Fractal simplex noise: `octaves` layers, each twice as fine and half
/// as strong as the one before. About `-1..1`.
pub struct Fractal {
    sources: Vec<Simplex<2>>,
    frequency: f64,
    /// Weight of each octave relative to the one before.
    gain: f64,
    /// Sum of the octaves' weights, to bring the result back to `-1..1`.
    norm: f64,
}

/// A seed of its own for each named field, so fields do not echo each other.
pub fn field_seed(seed: u64, name: &str) -> u64 {
    // FNV-1a over the name, mixed with the seed (splitmix64 finaliser).
    let mut h: u64 = 0xcbf29ce484222325;
    for b in name.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    let mut z = seed ^ h;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

impl Fractal {
    /// `wavelength`: metres of the coarsest octave.
    pub fn new(seed: u64, name: &str, octaves: usize, wavelength: f64) -> Self {
        Self::with_gain(seed, name, octaves, wavelength, 0.5)
    }

    /// With octaves weighted by `gain` each instead of a half: below a
    /// half, the fine octaves add less steepness than the coarse ones (at
    /// a half, every octave makes the slopes as steep as the first).
    pub fn with_gain(seed: u64, name: &str, octaves: usize, wavelength: f64, gain: f64) -> Self {
        let base = field_seed(seed, name);
        let sources = (0..octaves as u64)
            .map(|i| Source::simplex(base.wrapping_add(i.wrapping_mul(0x9E3779B97F4A7C15))))
            .collect();
        let norm = (0..octaves).map(|i| gain.powi(i as i32)).sum();
        Self {
            sources,
            frequency: 1.0 / wavelength,
            gain,
            norm,
        }
    }

    pub fn at(&self, x: f64, z: f64) -> f64 {
        let mut sum = 0.0;
        let (mut f, mut w) = (self.frequency, 1.0);
        for source in &self.sources {
            sum += w * source.sample([x * f, z * f]);
            f *= 2.0;
            w *= self.gain;
        }
        sum / self.norm
    }

    /// Ridges: `0..1`, 1 along crest lines that branch and wander,
    /// finer octaves adding side ridges mostly where the coarse ones
    /// already rise.
    pub fn ridges(&self, x: f64, z: f64) -> f64 {
        let mut sum = 0.0;
        let (mut f, mut w, mut weight) = (self.frequency, 1.0, 1.0);
        for source in &self.sources {
            // A crest, rounded off a little (smoothstep), so the slopes
            // below it are steepest halfway down, as on real mountains.
            let n = 1.0 - source.sample([x * f, z * f]).abs();
            let r = n * n * (3.0 - 2.0 * n) * weight;
            weight = (r * 1.5).clamp(0.0, 1.0);
            sum += w * r;
            f *= 2.0;
            w *= self.gain;
        }
        (sum / self.norm).clamp(0.0, 1.0)
    }
}

/// Fractal simplex noise in three dimensions (see `Fractal`). Its
/// gradient is about `GRADIENT / wavelength` long: a value `v` near zero
/// lies about `v * wavelength / GRADIENT` metres from the zero surface.
pub struct Fractal3 {
    /// Each octave with its own shift: simplex noise is zero on its whole
    /// lattice, so fields sampled unshifted would all be zero together
    /// there (tunnels, which run where two fields are zero, would only
    /// pass through those points).
    sources: Vec<(Simplex<3>, [f64; 3])>,
    frequency: f64,
    gain: f64,
    norm: f64,
}

impl Fractal3 {
    pub const GRADIENT: f64 = 2.5;

    pub fn new(seed: u64, name: &str, octaves: usize, wavelength: f64) -> Self {
        let base = field_seed(seed, name);
        let gain: f64 = 0.5;
        let sources = (0..octaves as u64)
            .map(|i| {
                let seed = base.wrapping_add(i.wrapping_mul(0x9E3779B97F4A7C15));
                let shift = std::array::from_fn(|k| {
                    (field_seed(seed, ["x", "y", "z"][k]) >> 11) as f64 / (1u64 << 53) as f64 * 1000.0
                });
                (Source::simplex(seed), shift)
            })
            .collect();
        let norm = (0..octaves).map(|i| gain.powi(i as i32)).sum();
        Self {
            sources,
            frequency: 1.0 / wavelength,
            gain,
            norm,
        }
    }

    pub fn at(&self, x: f64, y: f64, z: f64) -> f64 {
        let mut sum = 0.0;
        let (mut f, mut w) = (self.frequency, 1.0);
        for (source, [sx, sy, sz]) in &self.sources {
            sum += w * source.sample([x * f + sx, y * f + sy, z * f + sz]);
            f *= 2.0;
            w *= self.gain;
        }
        sum / self.norm
    }
}

