use cgmath::{Matrix4, Point3};
use glfw::MouseButton;

use crate::core::{
    mouse_picker::MousePicker,
    renderer::{
        line::Line,
        shader::{DynamicVertexArray, Shader, VertexAttributes},
        texture::Texture,
    },
    scene::Scene,
};

pub const CHUNK_SIZE: usize = 128;
pub const CHUNK_SIZE_FLOAT: f32 = CHUNK_SIZE as f32;

pub mod dual_contouring;
pub mod generator;
pub mod marching_cubes;
pub mod horizon;
pub mod materials;
pub mod scatter;
pub mod streaming;
mod terrain;
pub mod voxel;

pub struct Terrain<T: Chunk> {
    seed: u64,
    /// What shapes the world; the generator is made from it and the seed.
    world: generator::WorldSettings,
    /// Shapes the ground; shared with the loader threads and the chunks.
    generator: std::sync::Arc<generator::WorldGenerator>,
    /// Countdown until the chunks are rebuilt for an edited seed.
    reseed_in: Option<f64>,
    /// Models scattered over the chunks. Optional; see `scatter`.
    scatter: scatter::Scatter,
    /// Editor toggle: draw chunk and scatter colliders as wireframes.
    show_colliders: bool,
    /// The picture of the world the editor shows (see `WorldMap`).
    map: terrain::WorldMap,
    /// The ground beyond the chunks (see `horizon`), for chunk types that
    /// have one.
    horizon: horizon::Horizon,
    /// Loads the chunks around the camera (see `streaming`).
    streamer: streaming::Streamer<Vec<terrain::LoadedChunk<T>>>,
    shader: Shader,
    /// Draws the chunks in depth-only passes (see `depth_pass`), if the
    /// chunk type has such a program.
    depth_shader: Option<Shader>,
    textures: Vec<Texture>,
    mouse_picker: MousePicker,
}

pub trait Chunk {
    fn new(seed: u64, position: (f32, f32, f32), lod: usize) -> Self;
    /// The chunk at `position` (in chunks) as `generator` shapes the world.
    /// `lod` 0 is the full detail; above it a backend may leave out what
    /// only shows close up (the dual contouring chunks leave out caves and overhangs).
    /// Backends that make their own ground from the seed keep the default.
    fn generate(
        generator: &std::sync::Arc<generator::WorldGenerator>,
        position: (f32, f32, f32),
        lod: usize,
    ) -> Self
    where
        Self: Sized,
    {
        Self::new(generator.seed(), position, lod)
    }

    /// The chunks (by their `y`) of the column over chunk square
    /// `column` that may hold anything, in full detail or not (see
    /// `generate`); the others are not made. Backends whose ground lies in
    /// the bottom row keep the default.
    fn vertical_range(
        _generator: &generator::WorldGenerator,
        _column: (i32, i32),
        _detail: bool,
    ) -> std::ops::RangeInclusive<i32>
    where
        Self: Sized,
    {
        0..=0
    }

    /// Whether the terrain draws a horizon (see `horizon`) beyond these
    /// chunks: only for chunk types whose programs take its vertices.
    fn has_horizon() -> bool
    where
        Self: Sized,
    {
        false
    }

    /// Whether the chunk turned out to hold nothing to draw or collide
    /// with; such chunks are dropped right away.
    fn is_empty(&self) -> bool {
        false
    }
    fn buffer_data(&mut self);
    fn get_bounds(&self) -> ChunkBounds;
    /// A box around everything the chunk draws, for culling; the chunk's
    /// cube unless the type knows better.
    fn get_drawn_bounds(&self) -> ChunkBounds {
        self.get_bounds()
    }
    fn process_line(&mut self, line: &Line, button: &MouseButton) -> bool;
    fn get_position(&self) -> Point3<f32>;
    fn get_shader_source() -> (String, String);
    /// Vertex and fragment shader that draw the chunks' depth alone, for
    /// depth-only passes; `None` draws them with the shading program.
    fn get_depth_shader_source() -> Option<(String, String)> {
        None
    }
    /// Textures bound to units 0, 1, ... while the terrain draws. May set
    /// uniforms of `shader` that stay fixed.
    fn get_textures(shader: &Shader) -> Vec<Texture>;
    fn get_triangle_count(&self) -> usize;
    fn get_vertices(&self) -> Vec<[f32; 3]>;
    fn get_indices(&self) -> Vec<[u32; 3]>;

    /// Spots on this chunk where scattered models (trees, rocks, ...) may
    /// stand, as world-space `(x, y, z, yaw_radians)`. `attempts` random
    /// spots are tried and the unsuitable ones dropped; `salt` varies the
    /// pattern so each model gets its own. Must be deterministic for a given
    /// chunk, salt and attempt count. The default is empty — only terrain
    /// backends that support scattering implement this.
    fn scatter_points(&self, _salt: u64, _attempts: u32) -> Vec<(f32, f32, f32, f32)> {
        Vec::new()
    }

    /// Draw the chunk's ground. The terrain calls this for each of its
    /// chunks in view, with its `shader` bound and set up (uniforms,
    /// textures, `viewProjection`); `transform` is the chunk entity's,
    /// `distance` from the camera to the nearest point of the chunk (for
    /// its level of detail).
    /// Backends that draw themselves as components keep the default.
    fn draw(&self, _shader: &Shader, _transform: &Matrix4<f32>, _distance: f32) {}

    /// Optional transparent/overlay pass rendered after ALL chunks' opaque geometry.
    /// Override this to render water, glass, etc. Default is a no-op.
    fn render_transparent(
        &self,
        _scene: &Scene,
        _view_projection: &Matrix4<f32>,
        _parent_transform: &Matrix4<f32>,
    ) {}
}

pub struct ChunkMesh<T: VertexAttributes> {
    vertex_array: Option<DynamicVertexArray<T>>,
    indices: Option<Vec<u32>>,
    vertices: Vec<T>,
}

#[derive(Clone, Copy, Eq, PartialEq, Hash, Debug)]
pub struct ChunkBounds {
    pub min: (i32, i32, i32),
    pub max: (i32, i32, i32),
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgmath::{Point3, Vector3};

    #[test]
    fn parse_snaps_the_origin_to_the_chunk_at_the_origin() {
        let b = ChunkBounds::parse(Vector3::new(0.0, 0.0, 0.0));
        assert_eq!(b.min, (0, 0, 0));
        assert_eq!(
            b.max,
            (CHUNK_SIZE as i32, CHUNK_SIZE as i32, CHUNK_SIZE as i32)
        );
    }

    #[test]
    fn parse_wraps_a_point_just_past_a_boundary_into_the_next_chunk() {
        let just_over = CHUNK_SIZE_FLOAT + 0.5;
        let b = ChunkBounds::parse(Vector3::new(just_over, 0.0, 0.0));
        assert_eq!(b.min.0, CHUNK_SIZE as i32);
        assert_eq!(b.max.0, 2 * CHUNK_SIZE as i32);
    }

    #[test]
    fn parse_handles_negative_coordinates() {
        let b = ChunkBounds::parse(Vector3::new(-1.0, -1.0, -1.0));
        assert_eq!(b.min.0, -(CHUNK_SIZE as i32));
        assert_eq!(b.max.0, 0);
    }

    #[test]
    fn contains_matches_half_open_bounds() {
        let b = ChunkBounds::parse(Vector3::new(0.0, 0.0, 0.0));
        assert!(b.contains(Point3::new(1.0, 1.0, 1.0)));
        assert!(b.contains(Point3::new(0.0, 0.0, 0.0)));
        // The far wall is exclusive.
        assert!(!b.contains(Point3::new(CHUNK_SIZE_FLOAT, 0.0, 0.0)));
        assert!(!b.contains(Point3::new(-0.1, 0.0, 0.0)));
    }

    #[test]
    fn center_is_the_average_of_min_and_max() {
        let b = ChunkBounds::parse(Vector3::new(0.0, 0.0, 0.0));
        let c = b.center();
        assert!((c.x - CHUNK_SIZE_FLOAT / 2.0).abs() < 1e-5);
        assert!((c.y - CHUNK_SIZE_FLOAT / 2.0).abs() < 1e-5);
    }
}
