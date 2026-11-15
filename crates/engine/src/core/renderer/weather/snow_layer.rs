//! A real layer of snow on the ground around the camera: a grid raised off
//! the terrain by the snow's depth, with the trails pressed into it, so
//! footprints are real dents and what stands on the ground stands in the
//! snow.
//!
//! The grid is a set of square rings around the camera (a clipmap): the
//! finest has `FINEST` metres between its vertices, each next one twice
//! that and twice as wide. Every ring is the same mesh, moved in steps of
//! its coarser neighbour's spacing so the vertices stay put as the camera
//! moves; toward its edge a ring morphs into the coarser grid so they meet
//! without cracks, and leaves the inside to the finer one. The outermost
//! ring sinks into the ground at its edge, where the terrain's own
//! (painted) snow takes over. Heights come from the ground map (the terrain
//! seen from above, `exposure.rs`) and the trail map (`snow.rs`).

use cgmath::{Matrix4, Point3};
use gl::types::*;

use super::{WeatherMaps, WEATHER_MAPS_GLSL};
use crate::core::renderer::{
    pbr::{Lighting, LIGHTING_GLSL},
    shader::Shader,
    sky,
};

/// Cells per side of every ring (even, and twice even, so the rings line
/// up on their coarser neighbour's grid).
const CELLS: i32 = 128;
/// Metres between the finest ring's vertices (about a trail map texel).
const FINEST: f32 = 0.05;
/// Rings: the finest spans 6.4 m, the outermost about 100 m.
const LEVELS: u32 = 5;
/// Snow thinner than this (metres) is left to the terrain's painted snow.
const MIN_DEPTH: f32 = 0.02;
/// Height of the water's surface; no snow lies at or below it.
const WATER_LINE: f32 = crate::terrain::dual_contouring::WATER_LEVEL;
/// Uniforms the cloud field needs that `LIGHTING_GLSL` does not declare.
const CLOUD_UNIFORMS: &str = "uniform float uTime;\nuniform float cloudType;";

/// Metres around the camera (in x and in z) where the layer lies in full
/// for snow `depth` metres deep, 0 where it is not drawn at all. Inside,
/// the terrain leaves the trails to the layer.
pub fn full_reach(depth: f32, cover: f32) -> f32 {
    if depth < MIN_DEPTH || cover <= 0.0 {
        return 0.0;
    }
    let outer = FINEST * (1u32 << (LEVELS - 1)) as f32;
    // The outermost ring starts sinking at 0.55 of its half side, and its
    // centre may lie up to two spacings off the camera.
    CELLS as f32 * 0.5 * outer * 0.5 - 2.0 * outer
}

/// Snow kept under a footprint's bottom (metres), so the layer stays
/// clear of the terrain there.
const MIN_UNDER_PRINT: f32 = 0.03;

pub struct SnowLayer {
    shader: Shader,
    vao: GLuint,
    vbo: GLuint,
    ebo: GLuint,
    index_count: i32,
}

/// Centre (x, z) of ring `level` around `eye`, and metres between its
/// vertices. Moves in steps of twice the spacing, so a ring's vertices sit
/// on its coarser neighbour's grid.
fn ring(level: u32, eye: Point3<f32>) -> ([f32; 2], f32) {
    let spacing = FINEST * (1u32 << level) as f32;
    let step = spacing * 2.0;
    let snap = |v: f32| (v / step).round() * step;
    ([snap(eye.x), snap(eye.z)], spacing)
}

impl SnowLayer {
    pub fn new() -> Self {
        let shader = Shader::new(
            &format!(
                "#version 460 core\nuniform int isShadowPass;\n{WEATHER_MAPS_GLSL}\n{}",
                include_str!("snow_layer_vertex.glsl")
            ),
            &format!(
                "#version 460 core\n{LIGHTING_GLSL}\n{CLOUD_UNIFORMS}\n{}\n{}\n{}",
                sky::CLOUD_FIELD_GLSL,
                sky::CLOUD_SHADOW_GLSL,
                include_str!("snow_layer_fragment.glsl")
            ),
        );
        let side = CELLS + 1;
        let mut vertices = Vec::with_capacity((side * side * 2) as usize);
        for z in 0..side {
            for x in 0..side {
                vertices.push(x as f32);
                vertices.push(z as f32);
            }
        }
        let mut indices: Vec<u32> = Vec::with_capacity((CELLS * CELLS * 6) as usize);
        for z in 0..CELLS {
            for x in 0..CELLS {
                let i = (z * side + x) as u32;
                let (right, down) = (i + 1, i + side as u32);
                // Diagonals alternate, so morphed cells stay symmetric.
                if (x + z) % 2 == 0 {
                    indices.extend_from_slice(&[i, down, right, right, down, down + 1]);
                } else {
                    indices.extend_from_slice(&[i, down, down + 1, i, down + 1, right]);
                }
            }
        }
        let (mut vao, mut vbo, mut ebo) = (0, 0, 0);
        unsafe {
            gl::GenVertexArrays(1, &mut vao);
            gl::GenBuffers(1, &mut vbo);
            gl::GenBuffers(1, &mut ebo);
            gl::BindVertexArray(vao);
            gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
            gl::BufferData(
                gl::ARRAY_BUFFER,
                (vertices.len() * std::mem::size_of::<f32>()) as GLsizeiptr,
                vertices.as_ptr() as *const _,
                gl::STATIC_DRAW,
            );
            gl::BindBuffer(gl::ELEMENT_ARRAY_BUFFER, ebo);
            gl::BufferData(
                gl::ELEMENT_ARRAY_BUFFER,
                (indices.len() * std::mem::size_of::<u32>()) as GLsizeiptr,
                indices.as_ptr() as *const _,
                gl::STATIC_DRAW,
            );
            gl::EnableVertexAttribArray(0);
            gl::VertexAttribPointer(0, 2, gl::FLOAT, gl::FALSE, 0, std::ptr::null());
            gl::BindVertexArray(0);
        }
        Self {
            shader,
            vao,
            vbo,
            ebo,
            index_count: indices.len() as i32,
        }
    }

    /// Draw the layer around `eye` into the bound target, if enough snow
    /// lies and the ground map is there. The weather maps must be bound.
    pub fn render(
        &self,
        view_projection: &Matrix4<f32>,
        eye: Point3<f32>,
        lighting: &Lighting,
        maps: &WeatherMaps,
        cloud_type: f32,
    ) {
        let weather = &lighting.weather;
        if full_reach(weather.snow_depth, weather.snow_cover) <= 0.0 || maps.exposure.is_none() {
            return;
        }
        let shader = &self.shader;
        shader.bind();
        lighting.apply(shader);
        shader.set_uniform_mat4("viewProjection", view_projection);
        shader.set_uniform_1f("cells", CELLS as f32);
        shader.set_uniform_1f("waterLine", WATER_LINE);
        shader.set_uniform_1f("minUnderPrint", MIN_UNDER_PRINT);
        shader.set_uniform_1f("uTime", sky::seconds());
        shader.set_uniform_1f("cloudType", cloud_type);
        unsafe {
            // Whatever drew before may have left these otherwise.
            gl::Enable(gl::DEPTH_TEST);
            gl::DepthFunc(gl::LESS);
            gl::DepthMask(gl::TRUE);
            gl::Disable(gl::BLEND);
            gl::Disable(gl::CULL_FACE);
            gl::BindVertexArray(self.vao);
        }
        for level in 0..LEVELS {
            let (center, spacing) = ring(level, eye);
            shader.set_uniform_2f("levelCenter", center[0], center[1]);
            shader.set_uniform_1f("spacing", spacing);
            shader.set_uniform_1i("outermost", (level + 1 == LEVELS) as i32);
            if level == 0 {
                shader.set_uniform_1i("hasFiner", 0);
            } else {
                let (finer, finer_spacing) = ring(level - 1, eye);
                shader.set_uniform_1i("hasFiner", 1);
                shader.set_uniform_2f("finerCenter", finer[0], finer[1]);
                // A little short of the finer ring's edge: the two overlap
                // there (as the same surface) instead of leaving a seam.
                let half = CELLS as f32 * 0.5 * finer_spacing - finer_spacing * 0.5;
                shader.set_uniform_1f("finerHalf", half);
            }
            unsafe {
                gl::DrawElements(
                    gl::TRIANGLES,
                    self.index_count,
                    gl::UNSIGNED_INT,
                    std::ptr::null(),
                );
            }
        }
        unsafe { gl::BindVertexArray(0) };
    }
}

impl Drop for SnowLayer {
    fn drop(&mut self) {
        unsafe {
            gl::DeleteBuffers(1, &self.vbo);
            gl::DeleteBuffers(1, &self.ebo);
            gl::DeleteVertexArrays(1, &self.vao);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rings_sit_on_their_coarser_neighbours_grid() {
        let eye = Point3::new(12.37, 50.0, -7.91);
        for level in 1..LEVELS {
            let (finer, finer_spacing) = ring(level - 1, eye);
            let (coarser, spacing) = ring(level, eye);
            for axis in 0..2 {
                // The finer ring's edge lies on a vertex line of this one.
                let edge = finer[axis] + CELLS as f32 * 0.5 * finer_spacing;
                let lines = (edge - coarser[axis]) / spacing;
                assert!((lines - lines.round()).abs() < 1e-3, "level {level}");
                // And the finer ring lies inside this one.
                let reach = (finer[axis] - coarser[axis]).abs() + CELLS as f32 * 0.5 * finer_spacing;
                assert!(reach < CELLS as f32 * 0.5 * spacing, "level {level}");
            }
        }
    }
}
