//! The terrain: a world's ground streamed in chunks around the camera,
//! meshed, drawn with the horizon beyond and water over it, with models
//! scattered on it.
//!
//! What the world is (where the ground is, what it looks like, rivers,
//! roads, biomes) is not decided here but by a `TerrainSource` (see
//! `source`): the engine's own `hills`, or one a game's scripts register.
//! The engine brings the frame: streaming, meshing (`mesher`), levels of
//! detail, colliders, the horizon, water, scattered models and drawing.

pub const CHUNK_SIZE: usize = 128;
pub const CHUNK_SIZE_FLOAT: f32 = CHUNK_SIZE as f32;

pub mod chunk;
pub mod columns;
pub mod far_scatter;
pub mod generators;
pub mod hills;
pub mod horizon;
pub mod materials;
pub mod mesher;
pub mod noise;
pub mod scatter;
pub mod shaders;
pub mod source;
pub mod streaming;
mod terrain;
pub mod water;

pub use chunk::TerrainChunk;
pub use source::{
    ChunkCoord, ChunkField, ChunkSurface, HorizonSample, RiverPiece, SurfaceBand, SurfacePoint, SurfaceSample, TerrainGenerator,
    TerrainShaders, TerrainSource,
};
pub use terrain::Terrain;

use crate::core::renderer::shader::{DynamicVertexArray, VertexAttributes};

/// Where the far water (see `horizon`) is drawn: not on the columns whose
/// chunks are loaded (`loaded`: the picture of them, see `columns`, and its
/// first column), nor within `hole` metres of the camera at `centre`
/// (world x, z; under the ground, see the horizon's). `load` > 0 marks it
/// as the far water.
#[derive(Clone, Copy, Debug)]
pub struct FarWater {
    pub centre: [f32; 2],
    pub load: f32,
    pub hole: f32,
    pub loaded: (u32, (i32, i32)),
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
