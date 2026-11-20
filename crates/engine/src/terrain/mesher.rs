//! Meshing a chunk's ground: dual contouring (see `dual_contouring`) or
//! surface nets over its density field (see `source::ChunkField`) at every
//! level of detail, with the vertices on the full-detail surface, normals
//! from the full-detail field and skirts along the chunk's sides. Nothing here knows what the ground is
//! made of: the source paints every vertex (`ChunkSurface::surface`).

use std::collections::HashMap;

use fast_surface_nets::{
    ndshape::{RuntimeShape, Shape},
    surface_nets, SurfaceNetsBuffer,
};
use gl::types::GLuint;

use crate::core::renderer::shader::VertexAttributes;

use super::{
    source::{ChunkField, ChunkSurface, SurfacePoint, FIELD_MARGIN, FIELD_SPAN, SURFACE_DATA},
    CHUNK_SIZE, CHUNK_SIZE_FLOAT,
};

/// How the ground's density field becomes triangles. Dual contouring by
/// default; `FERRITE_TERRAIN_MESHER=surface_nets` brings back surface nets
/// (to compare the two).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mesher {
    DualContouring,
    SurfaceNets,
}

impl Mesher {
    pub fn in_use() -> Mesher {
        static IN_USE: std::sync::OnceLock<Mesher> = std::sync::OnceLock::new();
        *IN_USE.get_or_init(|| match std::env::var("FERRITE_TERRAIN_MESHER").as_deref() {
            Ok("surface_nets") => Mesher::SurfaceNets,
            _ => Mesher::DualContouring,
        })
    }
}

/// Levels of detail of the ground: the field meshed from samples 1, 2, 4
/// and 8 metres apart.
pub const LODS: usize = 4;
/// How far past a chunk's floor and ceiling its meshes may reach: the
/// coarsest level's step, twice (see `source::vertical_range_for`).
pub const REACH: f32 = 2.0 * (1 << (LODS - 1)) as f32;
/// How deep the skirts under a chunk's edges reach: this, plus this per
/// metre between the level's grid points.
const SKIRT_DEPTH: f32 = 4.0;
const SKIRT_DEPTH_PER_STEP: f32 = 2.0;

/// A vertex of the ground, as the terrain's programs take it (see
/// `shaders::STANDARD_VERTEX`).
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct TerrainVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    /// The ground's colour (display space).
    pub color: [f32; 3],
    /// The source's own numbers (see `SurfaceSample`).
    pub data: [f32; SURFACE_DATA],
}

impl VertexAttributes for TerrainVertex {
    fn get_vertex_attributes() -> Vec<(usize, GLuint)> {
        vec![(3, gl::FLOAT), (3, gl::FLOAT), (3, gl::FLOAT), (4, gl::FLOAT), (4, gl::FLOAT), (4, gl::FLOAT), (4, gl::FLOAT)]
    }
}

/// A chunk's ground meshed: every level of detail, finest first, each with
/// its skirt (chunk-local vertices and triangles).
pub struct MeshedChunk {
    pub levels: Vec<(Vec<TerrainVertex>, Vec<u32>)>,
    /// How many indices of the finest level are ground; its skirt follows.
    pub ground_indices: usize,
}

/// The ground of `surface`'s chunk at every level of detail (see
/// `MeshedChunk`); no levels where the chunk holds no surface. The field is
/// a volume, so overhangs and caves mesh as well as hills. The normals of
/// every level come from the full-detail field, so the light on the ground
/// does not change when a chunk switches levels. Each mesh has a skirt
/// along its open edges, pushed into the ground, deeper the coarser the
/// level, which hides the gaps where a neighbour drawn at another level
/// meets it.
pub fn mesh_chunk(surface: &dyn ChunkSurface) -> MeshedChunk {
    let field = DensityField { field: surface.field() };
    let empty = MeshedChunk {
        levels: Vec::new(),
        ground_indices: 0,
    };
    if field.field.is_solid() {
        return empty;
    }
    let mut ground_indices = 0;
    let mut levels = Vec::with_capacity(LODS);
    // The next level keeps the sharp detail, coarsely: it meets the finest
    // level along chunk sides, and without the detail the ground would
    // step there by as far as the detail reaches out or in.
    // The coarsest levels, drawn beyond where chunks are made in detail
    // (see `streaming::StreamRadius`), as a chunk made without detail is
    // drawn there: the field as seen from afar (`ChunkField::far_field`),
    // without caves. Those sampled that coarsely would break through the
    // rock over them as dark holes, and only where a chunk happened to be
    // made in detail before.
    let plain = field.field.far_field();
    let far = DensityField { field: plain.as_ref().unwrap_or(field.field) };
    for lod in 0..LODS {
        let step = 1 << lod;
        let field = if lod < 2 { &field } else { &far };
        let (mut vertices, mut indices) = match field.mesh_level(step, surface) {
            Some(mesh) => mesh,
            // No ground in this chunk.
            None if lod == 0 => return empty,
            // Caves too small for this level.
            None => (Vec::new(), Vec::new()),
        };
        if lod == 0 {
            ground_indices = indices.len();
        }
        let depth = SKIRT_DEPTH + SKIRT_DEPTH_PER_STEP * step as f32;
        add_skirt(&mut vertices, &mut indices, |p, n| field.solid_depth(p, n, depth), step as f32);
        levels.push((vertices, indices));
    }
    MeshedChunk { levels, ground_indices }
}

/// The finest level of `surface`'s chunk alone, unpainted and without a
/// skirt: the ground of `mesh_chunk`'s finest level as the collider takes
/// it, for where nothing is drawn (a server without a GPU). Its vertices
/// carry their positions only.
pub fn mesh_collider(surface: &dyn ChunkSurface) -> MeshedChunk {
    let field = DensityField { field: surface.field() };
    let shape = (!field.field.is_solid()).then(|| field.mesh_level_shape(1, surface)).flatten();
    let Some((positions, indices)) = shape else {
        return MeshedChunk {
            levels: Vec::new(),
            ground_indices: 0,
        };
    };
    let vertices = positions
        .into_iter()
        .map(|position| TerrainVertex { position, ..TerrainVertex::default() })
        .collect();
    MeshedChunk {
        ground_indices: indices.len(),
        levels: vec![(vertices, indices)],
    }
}

/// The density around one chunk (see `ChunkField`), and what meshing asks
/// of it between its whole metres.
pub struct DensityField<'a> {
    pub field: &'a ChunkField,
}

impl DensityField<'_> {
    const MARGIN: i32 = FIELD_MARGIN;
    const SPAN: i32 = FIELD_SPAN;

    #[inline]
    pub(super) fn at(&self, x: i32, y: i32, z: i32) -> f32 {
        self.field.at(x, y, z)
    }

    /// The field at chunk-local `p`, blended from the eight whole-metre
    /// points around it.
    pub fn value(&self, p: [f32; 3]) -> f32 {
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

    /// How far from chunk-local `p` the ground stays solid going against the
    /// normal `n`, up to `limit` metres (less a little, so what reaches
    /// that far stays inside): where rock is thin (beside a tunnel) a skirt
    /// hung that deep would come out on the other side.
    pub fn solid_depth(&self, p: [f32; 3], n: [f32; 3], limit: f32) -> f32 {
        const STEP: f32 = 0.5;
        // Within the field (the column's margins around the chunk).
        let (low, high) = (1.0 - Self::MARGIN as f32, (Self::SPAN - Self::MARGIN - 2) as f32);
        let mut t = STEP;
        while t <= limit {
            let at: [f32; 3] = std::array::from_fn(|i| {
                let c = p[i] - n[i] * t;
                if i == 1 { c } else { c.clamp(low, high) }
            });
            if self.value(at) > 0.0 {
                return (t - 2.0 * STEP).max(STEP * 0.5);
            }
            t += STEP;
        }
        limit
    }

    /// `p` moved onto the full-detail surface, along the gradient (a few
    /// Newton steps, at most `limit` metres): a coarse level's vertices
    /// then lie on the same ground as the finest level's, not on the
    /// coarse samples' approximation of it.
    fn onto_surface(&self, mut p: [f32; 3], limit: f32) -> [f32; 3] {
        let start = p;
        let (low, high) = (1.0 - Self::MARGIN as f32, (Self::SPAN - Self::MARGIN - 2) as f32);
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
    pub fn gradient(&self, p: [f32; 3]) -> [f32; 3] {
        // The last base whose far corner still has a neighbour beyond it.
        let (first, last) = (1 - Self::MARGIN, Self::SPAN - Self::MARGIN - 3);
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
    pub fn surface_nets(&self, step: i32, surface: &dyn ChunkSurface) -> Option<(Vec<TerrainVertex>, Vec<u32>)> {
        let (positions, indices) = self.surface_nets_shape(step)?;
        let vertices = positions.into_iter().map(|p| self.painted(p, surface)).collect();
        Some((vertices, indices))
    }

    /// `surface_nets`'s vertex positions and triangles, unpainted.
    fn surface_nets_shape(&self, step: i32) -> Option<(Vec<[f32; 3]>, Vec<u32>)> {
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
        let positions = buffer
            .positions
            .iter()
            .map(|p| self.onto_surface(p.map(|c| (c - 1.0) * step as f32), step as f32))
            .collect();
        Some((positions, buffer.indices))
    }

    /// The ground's level of detail `step` (metres between samples), by
    /// the mesher in use (see `Mesher`).
    pub fn mesh_level(&self, step: i32, surface: &dyn ChunkSurface) -> Option<(Vec<TerrainVertex>, Vec<u32>)> {
        match Mesher::in_use() {
            Mesher::DualContouring => self.dual_contouring(step, surface),
            Mesher::SurfaceNets => self.surface_nets(step, surface),
        }
    }

    /// `mesh_level`'s vertex positions and triangles, unpainted.
    pub fn mesh_level_shape(&self, step: i32, surface: &dyn ChunkSurface) -> Option<(Vec<[f32; 3]>, Vec<u32>)> {
        match Mesher::in_use() {
            Mesher::DualContouring => self.dual_contouring_shape(step, surface),
            Mesher::SurfaceNets => self.surface_nets_shape(step),
        }
    }

    /// The vertex at chunk-local `position` on the surface: its normal from
    /// the full-detail field and what the source paints there.
    pub(super) fn painted(&self, position: [f32; 3], surface: &dyn ChunkSurface) -> TerrainVertex {
        let origin = self.field.coord.origin();
        let floor = self.field.coord.floor();
        let g = self.gradient(position);
        let length = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt().max(1e-6);
        let normal = g.map(|c| c / length);
        let (cx, cz) = (position[0].round() as i32, position[2].round() as i32);
        let depth = self.field.height(cx, cz) - (floor + position[1]);
        let sample = surface.surface(&SurfacePoint {
            local: position,
            world: std::array::from_fn(|i| origin[i] + position[i] as f64),
            normal,
            depth,
        });
        TerrainVertex {
            position,
            normal,
            color: sample.color,
            data: sample.data,
        }
    }
}

/// Hang a skirt from the open edges of a mesh along the sides of the cube
/// it meshed (edges of one triangle only, which surface nets leaves there;
/// `step` the metres between its samples): each edge again, moved into the
/// ground against its normal as far as `depth(position, normal)` says,
/// joined to the edge by quads facing both ways.
pub fn add_skirt(
    vertices: &mut Vec<TerrainVertex>,
    indices: &mut Vec<u32>,
    depth: impl Fn([f32; 3], [f32; 3]) -> f32,
    step: f32,
) {
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
    let mut lower = |vertices: &mut Vec<TerrainVertex>, index: u32| {
        *lowered.entry(index).or_insert_with(|| {
            let mut v = vertices[index as usize];
            let depth = depth(v.position, v.normal);
            for i in 0..3 {
                v.position[i] -= v.normal[i] * depth;
            }
            vertices.push(v);
            vertices.len() as u32 - 1
        })
    };
    // Only edges along a side of the chunk, where a neighbour drawn at
    // another level meets it: thin rock (beside a tunnel, say) leaves edges
    // that count as open inside the chunk too, and a skirt there would
    // stick out of the ground.
    let band = 0.5 * step;
    let side = |p: [f32; 3], q: [f32; 3]| {
        [0, 2].iter().any(|&i| {
            (p[i] <= band && q[i] <= band) || (p[i] >= CHUNK_SIZE_FLOAT - band && q[i] >= CHUNK_SIZE_FLOAT - band)
        })
    };
    let mut edges: Vec<(u32, u32)> = open
        .into_keys()
        .filter(|&(a, b)| side(vertices[a as usize].position, vertices[b as usize].position))
        .collect();
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
