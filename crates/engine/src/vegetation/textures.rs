//! Textures drawn in code, for a tree that has none of its own: bark, a
//! twig of broad leaves, a sprig of needles and a palm's leaflets. RGBA,
//! the leaves cut out by their alpha.

use std::f32::consts::{PI, TAU};

use super::tree::{CrownShape, Foliage};

/// An RGBA image, `size` by `size`, rows from the top.
pub struct Image {
    pub size: u32,
    pub pixels: Vec<u8>,
}

impl Image {
    fn new(size: u32, fill: [u8; 4]) -> Self {
        Self {
            size,
            pixels: fill.repeat((size * size) as usize),
        }
    }

    fn set(&mut self, x: i32, y: i32, color: [u8; 4]) {
        let s = self.size as i32;
        if x < 0 || y < 0 || x >= s || y >= s {
            return;
        }
        let i = ((y * s + x) * 4) as usize;
        self.pixels[i..i + 4].copy_from_slice(&color);
    }
}

/// The kind of leaves to draw for a tree with `foliage` and these many
/// levels of branches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeafKind {
    Broad,
    Needles,
    Palm,
}

impl LeafKind {
    /// What fits a tree: fronds atop a trunk (an umbrella crown) are a
    /// palm's, others a conifer's sprays of needles; cards are broad
    /// leaves.
    pub fn for_tree(foliage: Foliage, crown: CrownShape) -> Self {
        match (foliage, crown) {
            (Foliage::Fronds, CrownShape::Umbrella) => Self::Palm,
            (Foliage::Fronds, _) => Self::Needles,
            _ => Self::Broad,
        }
    }
}

/// A smooth value noise, about `-1..1`, wrapping every `period` (so the
/// texture tiles).
fn noise(x: f32, y: f32, period: u32, seed: u32) -> f32 {
    let hash = |i: i32, j: i32| {
        let (i, j) = (i.rem_euclid(period as i32) as u32, j.rem_euclid(period as i32) as u32);
        let mut h = i.wrapping_mul(374761393) ^ j.wrapping_mul(668265263) ^ seed.wrapping_mul(2246822519);
        h = (h ^ (h >> 13)).wrapping_mul(1274126177);
        ((h ^ (h >> 16)) & 0xFFFF) as f32 / 32767.5 - 1.0
    };
    let (i, j) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - i as f32, y - j as f32);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let top = hash(i, j) + (hash(i + 1, j) - hash(i, j)) * sx;
    let bottom = hash(i, j + 1) + (hash(i + 1, j + 1) - hash(i, j + 1)) * sx;
    top + (bottom - top) * sy
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [u8; 4] {
    let c = |i: usize| ((a[i] + (b[i] - a[i]) * t.clamp(0.0, 1.0)) * 255.0).round() as u8;
    [c(0), c(1), c(2), 255]
}

/// Brown bark in furrows running up the trunk; tiles both ways.
pub fn bark() -> Image {
    let size = 256;
    let mut image = Image::new(size, [0; 4]);
    let (dark, light) = ([0.17, 0.12, 0.08], [0.42, 0.33, 0.24]);
    for y in 0..size {
        for x in 0..size {
            let (u, v) = (x as f32 / size as f32, y as f32 / size as f32);
            // Ridges up the trunk, wavering and splitting, between dark
            // furrows, crossed by a finer grain.
            let waver = 1.6 * noise(u * 6.0, v * 4.0, 6, 1) + 0.6 * noise(u * 12.0, v * 9.0, 12, 3);
            let ridge = (TAU * (u * 7.0 + 0.25 * waver)).sin().abs().powf(0.6);
            let crack = (0.5 + 0.5 * noise(u * 14.0, v * 20.0, 14, 4)).powi(3);
            let grain = 0.5 + 0.5 * noise(u * 64.0, v * 24.0, 64, 2);
            let t = 0.1 + 0.65 * ridge * (1.0 - 0.6 * crack) + 0.2 * grain;
            image.set(x as i32, y as i32, mix(dark, light, t));
        }
    }
    image
}

/// An ellipse-ish leaf: is `(x, y)` (in the leaf's frame: `x` along it from
/// its stalk, 0 to `length`, `y` across) inside it?
fn in_leaf(x: f32, y: f32, length: f32, width: f32) -> bool {
    if x < 0.0 || x > length {
        return false;
    }
    let t = x / length;
    // Rounded at the stalk, pointed at the tip.
    let half = 0.5 * width * (PI * t).sin().powf(0.8) * (1.0 - 0.3 * t);
    y.abs() < half
}

/// The leaves of `kind`: on a card (broad leaves, needles) a twig with
/// leaves either side growing up from its foot, on a frond (needles, palm)
/// leaflets either side of a midrib running up the middle.
pub fn leaves(kind: LeafKind) -> Image {
    let size = 256u32;
    let mut image = Image::new(size, [40, 70, 30, 0]);
    let s = size as f32;
    let (dark, light) = match kind {
        LeafKind::Broad => ([0.13, 0.27, 0.07], [0.33, 0.52, 0.16]),
        LeafKind::Needles => ([0.06, 0.18, 0.08], [0.16, 0.34, 0.15]),
        LeafKind::Palm => ([0.16, 0.30, 0.08], [0.38, 0.55, 0.18]),
    };
    let stalk = [0.28, 0.22, 0.12];
    // Pixel rows from the top; the texture's foot (v = 0) is the bottom row.
    for py in 0..size {
        for px in 0..size {
            let (u, v) = ((px as f32 + 0.5) / s, 1.0 - (py as f32 + 0.5) / s);
            let shade = 0.5 + 0.5 * noise(u * 24.0, v * 24.0, 24, 7);
            let color = match kind {
                LeafKind::Broad => {
                    // A twig up the middle, leaves either side angled up.
                    let mut hit = None;
                    if (u - 0.5).abs() < 0.012 && v < 0.92 {
                        hit = Some(mix(stalk, stalk, 0.0));
                    }
                    for k in 0..6 {
                        let at = 0.08 + 0.13 * k as f32;
                        for side in [-1.0f32, 1.0] {
                            let angle = side * 1.0;
                            let (dx, dy) = (u - 0.5, v - at);
                            // Into the leaf's frame.
                            let along = dx * angle.sin() + dy * angle.cos();
                            let across = dx * angle.cos() - dy * angle.sin();
                            let length = 0.3 - 0.02 * k as f32;
                            if in_leaf(along, across, length, length * 0.5) {
                                let vein = (across.abs() < 0.006) as i32 as f32;
                                hit = Some(mix(dark, light, shade * (1.0 - 0.5 * vein) + 0.15 * (along / length)));
                            }
                        }
                    }
                    // The leaf at the tip.
                    if in_leaf(v - 0.62, u - 0.5, 0.36, 0.2) {
                        hit = Some(mix(dark, light, shade));
                    }
                    hit
                }
                LeafKind::Needles | LeafKind::Palm => {
                    let palm = kind == LeafKind::Palm;
                    let midrib = if palm { 0.018 } else { 0.012 };
                    if (u - 0.5).abs() < midrib {
                        Some(mix(stalk, light, 0.3))
                    } else {
                        // Leaflets: thin, slanting toward the tip, out to
                        // the edges, close together.
                        let reach = (u - 0.5).abs() * 2.0;
                        // (Thick enough that the coarser mipmaps keep
                        // them: the cut-out is their alpha averaged.)
                        let slant = if palm { 0.55 } else { 0.3 };
                        let count = if palm { 26.0 } else { 24.0 };
                        let phase = (v - reach * slant) * count;
                        let thickness = if palm { 0.32 * (1.0 - 0.6 * reach) } else { 0.36 };
                        let inside = (phase - phase.round()).abs() < thickness && reach < 0.98;
                        inside.then(|| mix(dark, light, shade * 0.8 + 0.2 * (1.0 - reach)))
                    }
                }
            };
            if let Some(color) = color {
                image.set(px as i32, py as i32, color);
            }
        }
    }
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore]
    fn dump_textures() {
        let dir = std::env::var("DUMP").unwrap_or("/tmp".into());
        let save = |name: &str, image: Image| {
            image::RgbaImage::from_raw(image.size, image.size, image.pixels).unwrap().save(format!("{dir}/{name}.png")).unwrap();
        };
        save("bark", bark());
        save("broad", leaves(LeafKind::Broad));
        save("needles", leaves(LeafKind::Needles));
        save("palm", leaves(LeafKind::Palm));
    }

    #[test]
    fn leaves_are_cut_out_and_bark_is_opaque() {
        let opaque = |image: &Image| image.pixels.chunks(4).filter(|p| p[3] > 127).count() as f32 / (image.size * image.size) as f32;
        assert_eq!(opaque(&bark()), 1.0);
        for kind in [LeafKind::Broad, LeafKind::Needles, LeafKind::Palm] {
            let share = opaque(&leaves(kind));
            assert!(share > 0.1 && share < 0.8, "{kind:?}: {share} opaque");
        }
    }
}
