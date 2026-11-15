//! The ground as a volume: a density field that is negative inside the
//! ground and positive in the air, and the meshes of its surface at each
//! level of detail. Today the field is the terrain's height field (`y`
//! minus the ground's height), but nothing here assumes that: overhangs and
//! caves only need `DensityField::at` to say so.

use std::collections::HashMap;

use fast_surface_nets::{
    ndshape::{RuntimeShape, Shape},
    surface_nets, SurfaceNetsBuffer,
};

use super::{DualContouringChunk, Vertex};
use crate::terrain::{CHUNK_SIZE, CHUNK_SIZE_FLOAT};

/// The density around one chunk at whole metres, from one metre before
/// its corner to one past where the coarsest level samples.
pub(super) struct DensityField {
    /// Ground height per column, `(x, z)` from `MARGIN` before the corner.
    heights: Vec<f32>,
    span: i32,
    /// The chunk's floor, in world metres.
    floor: f32,
}

/// Columns before the chunk's corner (for the normals at its edge).
const MARGIN: i32 = 1;

impl DensityField {
    /// The field for `chunk`, far enough for levels up to `coarsest`
    /// metres between samples.
    pub(super) fn new(chunk: &DualContouringChunk, coarsest: i32) -> Self {
        // A level samples up to one step past the chunk, and the normals
        // look one metre further.
        let span = CHUNK_SIZE as i32 + coarsest + 2 + MARGIN;
        let mut heights = Vec::with_capacity((span * span) as usize);
        for z in -MARGIN..span - MARGIN {
            for x in -MARGIN..span - MARGIN {
                heights.push(chunk.column_noise(x, z) * CHUNK_SIZE_FLOAT - 1.0);
            }
        }
        Self {
            heights,
            span,
            floor: chunk.position.1 * CHUNK_SIZE_FLOAT,
        }
    }

    /// Density at chunk-local `(x, y, z)`, in metres from the surface
    /// (roughly): negative inside the ground.
    pub(super) fn at(&self, x: i32, y: i32, z: i32) -> f32 {
        let column = (z + MARGIN) * self.span + (x + MARGIN);
        self.floor + y as f32 - self.heights[column as usize]
    }

    /// The field's gradient at chunk-local `p`: central differences at the
    /// eight whole-metre points around it, blended. Points outward.
    pub(super) fn gradient(&self, p: [f32; 3]) -> [f32; 3] {
        // The last base whose far corner still has a neighbour beyond it.
        let last = self.span - MARGIN - 3;
        let base = p.map(|c| (c.floor() as i32).clamp(0, last));
        let t: [f32; 3] =
            std::array::from_fn(|i| (p[i] - base[i] as f32).clamp(0.0, 1.0));
        let mut sum = [0.0f32; 3];
        for corner in 0..8 {
            let (dx, dy, dz) = (corner & 1, (corner >> 1) & 1, (corner >> 2) & 1);
            let (x, y, z) = (base[0] + dx, base[1] + dy, base[2] + dz);
            let weight = if dx == 1 { t[0] } else { 1.0 - t[0] }
                * if dy == 1 { t[1] } else { 1.0 - t[1] }
                * if dz == 1 { t[2] } else { 1.0 - t[2] };
            if weight == 0.0 {
                continue;
            }
            let g = [
                self.at(x + 1, y, z) - self.at(x - 1, y, z),
                self.at(x, y + 1, z) - self.at(x, y - 1, z),
                self.at(x, y, z + 1) - self.at(x, y, z - 1),
            ];
            for i in 0..3 {
                sum[i] += weight * g[i];
            }
        }
        sum
    }

    /// The surface of the chunk's cube sampled `step` metres apart, with
    /// one sample past its far sides so it meets the neighbours: vertices
    /// (chunk-local, normals from the full-detail field) and triangles.
    /// `None` where the cube holds no surface.
    pub(super) fn surface_nets(&self, step: i32) -> Option<(Vec<Vertex>, Vec<u32>)> {
        let samples = (CHUNK_SIZE as i32 / step + 2) as u32;
        let shape = RuntimeShape::<u32, 3>::new([samples; 3]);
        let mut sdf = vec![0.0f32; shape.usize()];
        for (i, value) in sdf.iter_mut().enumerate() {
            let [x, y, z] = shape.delinearize(i as u32);
            *value = self.at(x as i32 * step, y as i32 * step, z as i32 * step);
        }
        let mut buffer = SurfaceNetsBuffer::default();
        surface_nets(&sdf, &shape, [0; 3], [samples - 1; 3], &mut buffer);
        if buffer.indices.is_empty() {
            return None;
        }
        let vertices = buffer
            .positions
            .iter()
            .map(|p| {
                let position = p.map(|c| c * step as f32);
                let g = self.gradient(position);
                let length = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt().max(1e-6);
                Vertex {
                    position,
                    normal: g.map(|c| c / length),
                    color: [0.0, 0.5, 0.1],
                }
            })
            .collect();
        Some((vertices, buffer.indices))
    }
}

/// Hang a skirt from the open edges of a mesh (edges of one triangle only,
/// which surface nets leaves at the sides of the cube it meshed): each edge
/// again, moved `depth` metres into the ground against its normal, joined
/// to the edge by quads facing both ways.
pub(super) fn add_skirt(vertices: &mut Vec<Vertex>, indices: &mut Vec<u32>, depth: f32) {
    // An inner edge appears in two triangles, once each way round.
    let mut open: HashMap<(u32, u32), ()> = HashMap::with_capacity(indices.len() / 8);
    for triangle in indices.chunks_exact(3) {
        for k in 0..3 {
            let (a, b) = (triangle[k], triangle[(k + 1) % 3]);
            if open.remove(&(b, a)).is_none() {
                open.insert((a, b), ());
            }
        }
    }
    let mut lowered: HashMap<u32, u32> = HashMap::with_capacity(open.len());
    let mut lower = |vertices: &mut Vec<Vertex>, index: u32| {
        *lowered.entry(index).or_insert_with(|| {
            let mut v = vertices[index as usize];
            for i in 0..3 {
                v.position[i] -= v.normal[i] * depth;
            }
            vertices.push(v);
            vertices.len() as u32 - 1
        })
    };
    let mut edges: Vec<(u32, u32)> = open.into_keys().collect();
    // The same mesh gets the same skirt.
    edges.sort_unstable();
    for (top0, top1) in edges {
        let (low0, low1) = (lower(vertices, top0), lower(vertices, top1));
        indices.extend_from_slice(&[
            top0, low0, top1, top1, low0, low1, // one side
            top0, top1, low0, top1, low1, low0, // the other
        ]);
    }
}
