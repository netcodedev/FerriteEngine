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
    generator::{
        bridges::Bridge,
        roads::{RoadSegment, Roads, TUNNEL_HALF_WIDTH, TUNNEL_WALL},
        ChunkCoord, Column, Underground3, WorldGenerator, COLUMN_MARGIN,
    },
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
    /// The tunnel tubes that reach into the chunk (see
    /// `Roads::tunnel_air`), and the chunk's corner in world metres.
    tunnels: Vec<RoadSegment>,
    /// The bridges that reach into the chunk (see `Bridge::solid`).
    bridges: Vec<Arc<Bridge>>,
    origin: [f64; 3],
    /// Where it has: the whole field at every whole metre the meshing
    /// reads (`span` per side from `-margin`, x fastest, then y), made
    /// once, since each vertex reads it a few hundred times.
    dense: Vec<f32>,
}

impl DensityField {
    /// The field of the chunk at `coord`, far enough for levels up to
    /// `coarsest` metres between samples.
    /// Without `detail`, the height field alone: no caves, no overhangs,
    /// no tunnels (what is drawn from afar).
    pub(super) fn new(generator: &WorldGenerator, coord: ChunkCoord, coarsest: i32, detail: bool) -> Self {
        // A level samples from one step before the chunk to one step past
        // it, and the normals look one metre further.
        assert!(coarsest < COLUMN_MARGIN, "the column is too narrow for steps of {coarsest} m");
        let origin = coord.origin();
        let floor = coord.y as f32 * CHUNK_SIZE_FLOAT;
        // The tubes that come within the field's reach.
        let reach = (COLUMN_MARGIN + 1) as f64 + TUNNEL_HALF_WIDTH as f64;
        let tunnels: Vec<RoadSegment> = if detail {
            generator
                .tunnels_near(coord.x, coord.z)
                .iter()
                .filter(|s| {
                    let (lo, hi) = (s.level[0].min(s.level[1]), s.level[0].max(s.level[1]));
                    let (bottom, top) = (floor - reach as f32, floor + CHUNK_SIZE_FLOAT + reach as f32);
                    let size = CHUNK_SIZE as f64;
                    hi + TUNNEL_WALL + TUNNEL_HALF_WIDTH >= bottom
                        && lo <= top
                        && s.a[0].max(s.b[0]) >= origin[0] - reach
                        && s.a[0].min(s.b[0]) <= origin[0] + size + reach
                        && s.a[1].max(s.b[1]) >= origin[2] - reach
                        && s.a[1].min(s.b[1]) <= origin[2] + size + reach
                })
                .copied()
                .collect()
        } else {
            Vec::new()
        };
        // Bridges at every level: they stand out over the land.
        let size = CHUNK_SIZE as f64;
        let bridges: Vec<Arc<Bridge>> = generator
            .bridges_near(coord.x, coord.z)
            .iter()
            .filter(|b| {
                let (lo, hi) = (b.lo(), b.hi());
                b.top + 1.0 >= floor - reach as f32
                    && hi[0] >= origin[0] - reach
                    && lo[0] <= origin[0] + size + reach
                    && hi[1] >= origin[2] - reach
                    && lo[1] <= origin[2] + size + reach
            })
            .cloned()
            .collect();
        let mut field = Self {
            column: generator.column(coord.x, coord.z),
            span: Column::SPAN,
            margin: COLUMN_MARGIN,
            floor,
            underground: generator.underground(coord, detail),
            tunnels,
            bridges,
            origin,
            dense: Vec::new(),
        };
        if field.underground.is_some() || !field.tunnels.is_empty() || !field.bridges.is_empty() {
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
        let tunnel = if self.tunnels.is_empty() {
            f32::NEG_INFINITY
        } else {
            let (wx, wz) = (self.origin[0] + x as f64, self.origin[2] + z as f64);
            Roads::tunnel_air(&self.tunnels, wx, self.floor + y as f32, wz)
        };
        let ground = match &self.underground {
            None => ground,
            Some(u) => {
                let [push, cave] = u.at(x, y, z);
                (ground - push).max(Roads::cave_beside_tunnel(cave, tunnel))
            }
        };
        let ground = ground.max(tunnel);
        if self.bridges.is_empty() {
            return ground;
        }
        let (wx, wz) = (self.origin[0] + x as f64, self.origin[2] + z as f64);
        let y = self.floor + y as f32;
        self.bridges.iter().map(|b| b.solid(wx, y, wz)).fold(ground, f32::min)
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
        self.underground.is_none()
            && self.tunnels.is_empty()
            && self.bridges.is_empty()
            && self.floor + CHUNK_SIZE_FLOAT + 12.0 < self.column.min_height
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

    /// Whether chunk-local `p` (on the surface) is a bridge's stone, and
    /// whether (its normal `up` high) on the road over it.
    fn on_bridge(&self, p: [f32; 3], up: f32) -> (bool, bool) {
        if self.bridges.is_empty() {
            return (false, false);
        }
        let (x, y, z) = (self.origin[0] + p[0] as f64, self.floor + p[1], self.origin[2] + p[2] as f64);
        let stone = self.bridges.iter().any(|b| b.covers(x, z, 0.8) && b.solid(x, y, z) < 0.6);
        let deck = stone && up > 0.7 && self.bridges.iter().any(|b| b.on_deck(x, y, z));
        (stone, deck)
    }

    /// Whether chunk-local `p` (on the surface, its normal `up` high) is
    /// on a tunnel's floor: facing up, with the tube right over it.
    fn on_tunnel_floor(&self, p: [f32; 3], up: f32) -> bool {
        if self.tunnels.is_empty() || up < 0.7 {
            return false;
        }
        let (x, z) = (self.origin[0] + p[0] as f64, self.origin[2] + p[2] as f64);
        Roads::tunnel_air(&self.tunnels, x, self.floor + p[1] + 1.0, z) > 0.5
    }

    /// How far from chunk-local `p` the ground stays solid going against the
    /// normal `n`, up to `limit` metres (less a little, so what reaches
    /// that far stays inside): where rock is thin (beside a tunnel) a skirt
    /// hung that deep would come out on the other side.
    pub(super) fn solid_depth(&self, p: [f32; 3], n: [f32; 3], limit: f32) -> f32 {
        const STEP: f32 = 0.5;
        // Within the field (the column's margins around the chunk).
        let (low, high) = (1.0 - self.margin as f32, (self.span - self.margin - 2) as f32);
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
                // A tunnel's floor is road (its walls and arch are rock), and
                // so is a bridge's deck (the rest of it stone).
                let (stone, deck) = self.on_bridge(position, g[1] / length);
                let inside = if stone && !deck { 1.0 } else { inside };
                let floor = deck || self.on_tunnel_floor(position, g[1] / length);
                // Roads, settlements and gravel lie on the surface only.
                let surface = if floor { 1.0 } else { 1.0 - inside };
                let [road, settlement, gravel] = self.column.paint(position[0], position[2]);
                let road = if floor { 1.0 } else { road };
                // Swamps are mud in patches, except where something lies.
                let mud = biome.swamp * (1.0 - road.max(settlement).max(gravel));
                Vertex {
                    position,
                    normal: g.map(|c| c / length),
                    color: std::array::from_fn(|k| biome.color[k] + (rock[k] - biome.color[k]) * inside),
                    climate: [
                        biome.temperature + (biome.temperature.max(4.0) - biome.temperature) * inside,
                        biome.humidity,
                    ],
                    cover: [biome.forest, biome.dry, biome.jungle, biome.tundra].map(|v| v * surface),
                    paint: [road, settlement, gravel, mud].map(|v| v * surface),
                    underground: if floor { 0.0 } else { inside },
                }
            })
            .collect();
        Some((vertices, buffer.indices))
    }
}

/// Hang a skirt from the open edges of a mesh along the sides of the cube
/// it meshed (edges of one triangle only, which surface nets leaves there;
/// `step` the metres between its samples): each edge again, moved into the
/// ground against its normal as far as `depth(position, normal)` says,
/// joined to the edge by quads facing both ways.
pub(super) fn add_skirt(
    vertices: &mut Vec<Vertex>,
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
    let mut lower = |vertices: &mut Vec<Vertex>, index: u32| {
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
