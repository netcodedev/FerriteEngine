//! The lattice of the clouds' value noise as a 3D texture: one random value
//! per lattice point, repeating every `SIZE` points. `cloudNoise3` reads it
//! with one filtered lookup, where it used to hash the eight corners of its
//! cell (see `cloud_field.glsl`).

use std::cell::RefCell;

use crate::core::gfx::{self, Format, Rect, SamplerDesc, TextureDesc, TextureKind, UploadFormat};

/// Lattice points along each side; must match `CLOUD_NOISE_SIZE` in
/// `cloud_field.glsl`.
pub const SIZE: usize = 128;
/// Texture unit the lattice stays bound to; must match the `binding` of
/// `cloudNoise` in `cloud_field.glsl`.
pub const UNIT: u32 = 17;
/// Linear between lattice points (the shader bends the position in the
/// cell by the hermite curve first), repeating.
const SAMPLER: SamplerDesc = SamplerDesc::LINEAR_REPEAT;

thread_local! {
    static TEXTURE: RefCell<Option<gfx::Texture>> = const { RefCell::new(None) };
}

/// `cloudHash13` of `cloud_field.glsl`: a value in 0..1 for lattice point `p`.
fn hash(p: [f32; 3]) -> f32 {
    let fract = |v: f32| v - v.floor();
    let mut p = [fract(p[0] * 0.1031), fract(p[1] * 0.1031), fract(p[2] * 0.1031)];
    let d = p[0] * (p[1] + 33.33) + p[1] * (p[2] + 33.33) + p[2] * (p[0] + 33.33);
    p = [p[0] + d, p[1] + d, p[2] + d];
    fract((p[0] + p[1]) * p[2])
}

/// The lattice's values, x fastest, as 16-bit fractions.
fn lattice() -> Vec<u16> {
    let mut values = Vec::with_capacity(SIZE * SIZE * SIZE);
    for z in 0..SIZE {
        for y in 0..SIZE {
            for x in 0..SIZE {
                let v = hash([x as f32, y as f32, z as f32]);
                values.push((v.clamp(0.0, 1.0) * 65535.0).round() as u16);
            }
        }
    }
    values
}

/// Bind the lattice to `UNIT`, made on first use. Call before drawing with
/// a shader that includes `cloud_field.glsl`.
pub fn bind() {
    if !gfx::is_drawing() {
        return;
    }
    TEXTURE.with(|texture| {
        let mut texture = texture.borrow_mut();
        let texture = texture.get_or_insert_with(|| {
            let size = SIZE as u32;
            let desc = TextureDesc {
                kind: TextureKind::D3,
                layers: size,
                ..TextureDesc::d2(Format::R16, size, size)
            };
            let texture = gfx::Texture::new(desc);
            texture.write(0, Rect::sized(size, size), 0, size, UploadFormat::R16, gfx::as_bytes(&lattice()));
            texture
        });
        gfx::bind_texture(UNIT, texture, &SAMPLER);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lattice_spreads_over_0_to_1() {
        let values = lattice();
        assert_eq!(values.len(), SIZE * SIZE * SIZE);
        let mean = values.iter().map(|&v| v as f64 / 65535.0).sum::<f64>() / values.len() as f64;
        assert!((mean - 0.5).abs() < 0.02, "{mean}");
        let low = values.iter().filter(|&&v| v < 6554).count() as f64 / values.len() as f64;
        assert!((low - 0.1).abs() < 0.02, "{low}");
    }
}
