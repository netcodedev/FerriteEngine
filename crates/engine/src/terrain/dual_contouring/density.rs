//! The ground as a volume: a density field that is negative inside the
//! ground and positive in the air, and the meshes of its surface at each
//! level of detail. Today the field is the terrain's height field (`y`
//! minus the ground's height), but nothing here assumes that: overhangs and
//! caves only need `DensityField::at` to say so.

use std::{collections::HashMap, sync::Arc};

use fast_surface_nets::{
    ndshape::{RuntimeShape, Shape},
    surface_nets, SurfaceNetsBuffer,
};

use super::Vertex;
use crate::terrain::{
    generator::{ChunkCoord, Column, Underground3, WorldGenerator, COLUMN_MARGIN},
    CHUNK_SIZE, CHUNK_SIZE_FLOAT,
};

/// The density around one chunk at whole metres, from one metre before
/// to one metre past where the coarsest level samples (a step beyond the
/// chunk on every side).
pub(super) struct DensityField {
    /// The ground's height over the chunk's square (see `Column`).
    column: Arc<Column>,
    span: i32,
    margin: i32,
    /// The chunk's floor, in world metres.
    floor: f32,
    /// Caves and overhangs, where the chunk has any.
    underground: Option<Underground3>,
    /// Where it has: the whole field at every whole metre the meshing
    /// reads (`span` per side from `-margin`, x fastest, then y), made
    /// once, since each vertex reads it a few hundred times.
    dense: Vec<f32>,
}

impl DensityField {
    /// The field of the chunk at `coord`, far enough for levels up to
    /// `coarsest` metres between samples.
    pub(super) fn new(generator: &WorldGenerator, coord: ChunkCoord, coarsest: i32) -> Self {
        // A level samples from one step before the chunk to one step past
        // it, and the normals look one metre further.
        assert!(coarsest < COLUMN_MARGIN, "the column is too narrow for steps of {coarsest} m");
        let mut field = Self {
            column: generator.column(coord.x, coord.z),
            span: Column::SPAN,
            margin: COLUMN_MARGIN,
            floor: coord.y as f32 * CHUNK_SIZE_FLOAT,
            underground: generator.underground(coord),
            dense: Vec::new(),
        };
        if field.underground.is_some() {
            let (span, margin) = (field.span, field.margin);
            let mut dense = Vec::with_capacity((span * span * span) as usize);
            for z in -margin..span - margin {
                for y in -margin..span - margin {
                    for x in -margin..span - margin {
                        dense.push(field.compute(x, y, z));
                    }
                }
            }
            field.dense = dense;
        }
        field
    }

    fn compute(&self, x: i32, y: i32, z: i32) -> f32 {
        let ground = self.floor + y as f32 - self.column.height(x, z);
        match &self.underground {
            None => ground,
            Some(u) => {
                let [push, cave] = u.at(x, y, z);
                (ground - push).max(cave)
            }
        }
    }

    /// Density at chunk-local `(x, y, z)`, in metres from the surface
    /// (roughly): negative inside the ground.
    #[inline]
    pub(super) fn at(&self, x: i32, y: i32, z: i32) -> f32 {
        if self.dense.is_empty() {
            return self.floor + y as f32 - self.column.height(x, z);
        }
        // Newton steps may overshoot the chunk's height a little.
        let i = |v: i32| (v + self.margin).clamp(0, self.span - 1);
        self.dense[((i(z) * self.span + i(y)) * self.span + i(x)) as usize]
    }

    /// Whether the chunk lies wholly in solid ground (under its surface,
    /// with no cave), so it has nothing to mesh.
    pub(super) fn is_solid(&self) -> bool {
        self.underground.is_none() && self.floor + CHUNK_SIZE_FLOAT + 12.0 < self.column.min_height
    }

    /// The field at chunk-local `p`, blended from the eight whole-metre
    /// points around it.
    pub(super) fn value(&self, p: [f32; 3]) -> f32 {
        let base = p.map(|c| c.floor() as i32);
        let t: [f32; 3] = std::array::from_fn(|i| p[i] - base[i] as f32);
        let mut sum = 0.0;
        for corner in 0..8 {
            let (dx, dy, dz) = (corner & 1, (corner >> 1) & 1, (corner >> 2) & 1);
            let weight = if dx == 1 { t[0] } else { 1.0 - t[0] }
                * if dy == 1 { t[1] } else { 1.0 - t[1] }
                * if dz == 1 { t[2] } else { 1.0 - t[2] };
            if weight != 0.0 {
                sum += weight * self.at(base[0] + dx, base[1] + dy, base[2] + dz);
            }
        }
        sum
    }

    /// `p` moved onto the full-detail surface, along the gradient (a few
    /// Newton steps, at most `limit` metres): a coarse level's vertices
    /// then lie on the same ground as the finest level's, not on the
    /// coarse samples' approximation of it.
    fn onto_surface(&self, mut p: [f32; 3], limit: f32) -> [f32; 3] {
        let start = p;
        let (low, high) = (1.0 - self.margin as f32, (self.span - self.margin - 2) as f32);
        for _ in 0..3 {
            let g = self.gradient(p).map(|c| c * 0.5);
            let g2 = g[0] * g[0] + g[1] * g[1] + g[2] * g[2];
            if g2 < 1e-8 {
                break;
            }
            let d = self.value(p) / g2;
            for i in 0..3 {
                p[i] -= g[i] * d;
            }
            let moved: f32 = (0..3).map(|i| (p[i] - start[i]).powi(2)).sum::<f32>().sqrt();
            if moved > limit {
                for i in 0..3 {
                    p[i] = start[i] + (p[i] - start[i]) * limit / moved;
                }
            }
            p[0] = p[0].clamp(low, high);
            p[2] = p[2].clamp(low, high);
        }
        p
    }

    /// The field's gradient at chunk-local `p`: central differences at the
    /// eight whole-metre points around it, blended. Points outward.
    pub(super) fn gradient(&self, p: [f32; 3]) -> [f32; 3] {
        // The last base whose far corner still has a neighbour beyond it.
        let (first, last) = (1 - self.margin, self.span - self.margin - 3);
        let base = p.map(|c| (c.floor() as i32).clamp(first, last));
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
    /// one sample past each side: vertices (chunk-local, normals from the
    /// full-detail field) and triangles. Surface nets puts its vertices
    /// inside the cells, so the mesh reaches about half a step past each
    /// side and overlaps its neighbours' whatever their level; without the
    /// sample before the near sides, a coarse mesh would start half its
    /// step inside the chunk and leave a gap a fine neighbour does not
    /// fill.
    /// `None` where the cube holds no surface.
    pub(super) fn surface_nets(&self, step: i32) -> Option<(Vec<Vertex>, Vec<u32>)> {
        let samples = (CHUNK_SIZE as i32 / step + 3) as u32;
        let shape = RuntimeShape::<u32, 3>::new([samples; 3]);
        let mut sdf = vec![0.0f32; shape.usize()];
        for (i, value) in sdf.iter_mut().enumerate() {
            let [x, y, z] = shape.delinearize(i as u32);
            let local = |i: u32| (i as i32 - 1) * step;
            *value = self.at(local(x), local(y), local(z));
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
                let position = self.onto_surface(p.map(|c| (c - 1.0) * step as f32), step as f32);
                let g = self.gradient(position);
                let length = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt().max(1e-6);
                let y = self.floor + position[1];
                let biome = self.column.biome(position[0], position[2], y);
                // Under the surface (in caves and under overhangs): bare
                // rock, out of the snow.
                let (cx, cz) = (position[0].round() as i32, position[2].round() as i32);
                let below = self.column.height(cx, cz) - y;
                let inside = ((below - 1.5) / 4.0).clamp(0.0, 1.0);
                let rock = [0.40, 0.37, 0.33];
                Vertex {
                    position,
                    normal: g.map(|c| c / length),
                    color: std::array::from_fn(|k| biome.color[k] + (rock[k] - biome.color[k]) * inside),
                    climate: [
                        biome.temperature + (biome.temperature.max(4.0) - biome.temperature) * inside,
                        biome.humidity,
                    ],
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
