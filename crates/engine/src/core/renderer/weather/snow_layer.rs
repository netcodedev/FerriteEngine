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

use super::{WeatherMaps, WEATHER_MAPS_GLSL};
use crate::core::{
    gfx,
    renderer::{
        pbr::{Lighting, LIGHTING_GLSL},
        shader::Shader,
        sky,
    },
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
/// Where the beach ends: no snow lies below the sea's level. Set by the
/// terrain from its world (see `TerrainSource::sea_level`).
static WATER_LINE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// The sea's level, below which no snow lies.
pub fn set_water_line(level: f32) {
    WATER_LINE.store(level.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

fn water_line() -> f32 {
    f32::from_bits(WATER_LINE.load(std::sync::atomic::Ordering::Relaxed))
}
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
    /// The grid's vertices (x, z in cells) and indices.
    vertices: gfx::Buffer,
    indices: gfx::Buffer,
    /// Indices of the whole grid, at the start of the index buffer.
    full_count: u32,
    /// Indices of the grid without its middle (`in_hole`), after them.
    ring_count: u32,
}

/// Cells (on each axis) of a ring that the next finer ring always covers,
/// wherever the two lie: the fragment shader discards all of their
/// fragments, so they are left out of the mesh. The finer ring's centre
/// is at most one spacing off this ring's (`ring`), and it takes over
/// within `finerHalf` (`CELLS / 4 - 1/4` spacings) of it: from
/// `CELLS / 2 + 1 - CELLS / 4 + 1/4` on, up to `CELLS / 2 - 1 + CELLS / 4 - 1/4`.
/// There the ring does not morph yet (that starts at 0.7 of its reach).
fn in_hole(cell: i32) -> bool {
    let quarter = CELLS / 4;
    let first = CELLS / 2 + 1 - quarter + 1;
    let last = CELLS / 2 - 1 + quarter - 2;
    (first..=last).contains(&cell)
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
        // The whole grid (the finest ring), then the same without its
        // middle (the others, where the finer ring draws instead), each
        // in the same order.
        let mut indices: Vec<u32> = Vec::with_capacity((CELLS * CELLS * 12) as usize);
        for skip_hole in [false, true] {
            for z in 0..CELLS {
                for x in 0..CELLS {
                    if skip_hole && in_hole(x) && in_hole(z) {
                        continue;
                    }
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
        }
        let full_count = (CELLS * CELLS * 6) as u32;
        Self {
            shader,
            vertices: gfx::Buffer::from_slice(gfx::BufferUsage::VERTEX, gfx::BufferMemory::Static, &vertices),
            indices: gfx::Buffer::from_slice(gfx::BufferUsage::INDEX, gfx::BufferMemory::Static, &indices),
            full_count,
            ring_count: indices.len() as u32 - full_count,
        }
    }

    /// The grid's vertices: x, z (in cells) at location 0.
    fn layout() -> gfx::VertexLayout {
        gfx::VertexLayout::packed(0, gfx::StepMode::Vertex, &[gfx::VertexFormat::F32x2])
    }

    /// Draw the layer around `eye` into the bound target, if enough snow
    /// lies and the ground map is there. The weather maps must be bound.
    /// `flat` leaves the trails out of the shape, for shadows and the
    /// ambient occlusion's depth: the camera's view shades them smoothly
    /// itself, and what the depth of a few centimetres of print would add
    /// there is mostly shimmer.
    pub fn render(
        &self,
        view_projection: &Matrix4<f32>,
        eye: Point3<f32>,
        lighting: &Lighting,
        maps: &WeatherMaps,
        cloud_type: f32,
        flat: bool,
    ) {
        let weather = &lighting.weather;
        if full_reach(weather.snow_depth, weather.snow_cover) <= 0.0 || maps.exposure.is_none() {
            return;
        }
        let shader = &self.shader;
        let desc = gfx::PipelineDesc {
            vertex_layouts: vec![Self::layout()],
            depth: gfx::DepthState::LEQUAL,
            ..Default::default()
        };
        gfx::set_pipeline(shader.pipeline(&desc));
        gfx::set_vertex_buffer(0, &self.vertices, 0, Self::layout().stride);
        gfx::set_index_buffer(&self.indices, gfx::IndexFormat::U32);
        lighting.apply(shader);
        shader.set_uniform_mat4("viewProjection", view_projection);
        shader.set_uniform_1f("cells", CELLS as f32);
        shader.set_uniform_1f("waterLine", water_line());
        shader.set_uniform_1f("minUnderPrint", MIN_UNDER_PRINT);
        shader.set_uniform_1i("flatLayer", flat as i32);
        shader.set_uniform_1f("uTime", sky::seconds());
        shader.set_uniform_1f("cloudType", cloud_type);
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
            // The finest ring whole; the others without the middle the
            // finer one covers.
            let (count, first) = if level == 0 {
                (self.full_count, 0)
            } else {
                (self.ring_count, self.full_count)
            };
            gfx::draw_indexed(first, count, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cells_left_out_of_a_ring_lie_where_the_finer_ring_draws() {
        let hole: Vec<i32> = (0..CELLS).filter(|&c| in_hole(c)).collect();
        assert_eq!(hole.len(), 60);
        for step in 0..2000 {
            let t = step as f32 * 0.137;
            let eye = Point3::new(t * 3.1 - 900.0, 0.0, 450.0 - t * 1.7);
            for level in 1..LEVELS {
                let (center, spacing) = ring(level, eye);
                let (finer, finer_spacing) = ring(level - 1, eye);
                // As `render` sets `finerHalf`.
                let half = CELLS as f32 * 0.5 * finer_spacing - finer_spacing * 0.5;
                for axis in 0..2 {
                    // A cell's corners, as `snow_layer_vertex.glsl` places
                    // them (no morph this close).
                    for corner in [hole[0], hole[hole.len() - 1] + 1] {
                        let p = center[axis] + (corner as f32 - CELLS as f32 * 0.5) * spacing;
                        assert!((p - finer[axis]).abs() < half - 0.5 * spacing, "level {level} eye {eye:?}");
                    }
                    let reach = CELLS as f32 * 0.5 * spacing;
                    let far = (hole[hole.len() - 1] + 1) as f32 - CELLS as f32 * 0.5;
                    assert!(far.abs() * spacing + (center[axis] - [eye.x, eye.z][axis]).abs() < 0.7 * reach);
                }
            }
        }
    }

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
