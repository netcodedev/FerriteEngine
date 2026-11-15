//! The lattice of the clouds' value noise as a 3D texture: one random value
//! per lattice point, repeating every `SIZE` points. `cloudNoise3` reads it
//! with one filtered lookup, where it used to hash the eight corners of its
//! cell (see `cloud_field.glsl`).

use std::cell::Cell;

use gl::types::{GLint, GLsizei, GLuint};

/// Lattice points along each side; must match `CLOUD_NOISE_SIZE` in
/// `cloud_field.glsl`.
pub const SIZE: usize = 128;
/// Texture unit the lattice stays bound to; must match the `binding` of
/// `cloudNoise` in `cloud_field.glsl`.
pub const UNIT: u32 = 17;

thread_local! {
    static TEXTURE: Cell<GLuint> = const { Cell::new(0) };
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
    if crate::core::headless::is_headless() {
        return;
    }
    let mut texture = TEXTURE.with(Cell::get);
    unsafe {
        gl::ActiveTexture(gl::TEXTURE0 + UNIT);
        if texture == 0 {
            gl::GenTextures(1, &mut texture);
            gl::BindTexture(gl::TEXTURE_3D, texture);
            let values = lattice();
            gl::PixelStorei(gl::UNPACK_ALIGNMENT, 2);
            gl::TexImage3D(
                gl::TEXTURE_3D,
                0,
                gl::R16 as GLint,
                SIZE as GLsizei,
                SIZE as GLsizei,
                SIZE as GLsizei,
                0,
                gl::RED,
                gl::UNSIGNED_SHORT,
                values.as_ptr() as *const _,
            );
            gl::PixelStorei(gl::UNPACK_ALIGNMENT, 4);
            // Linear between lattice points: the shader bends the position
            // in the cell by the hermite curve first.
            for (name, value) in [
                (gl::TEXTURE_MIN_FILTER, gl::LINEAR),
                (gl::TEXTURE_MAG_FILTER, gl::LINEAR),
                (gl::TEXTURE_WRAP_S, gl::REPEAT),
                (gl::TEXTURE_WRAP_T, gl::REPEAT),
                (gl::TEXTURE_WRAP_R, gl::REPEAT),
            ] {
                gl::TexParameteri(gl::TEXTURE_3D, name, value as GLint);
            }
            TEXTURE.with(|t| t.set(texture));
        } else {
            gl::BindTexture(gl::TEXTURE_3D, texture);
        }
        gl::ActiveTexture(gl::TEXTURE0);
    }
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
