//! The horizon: the ground beyond the loaded chunks, out to many
//! kilometres, as one height field mesh. It is made from the source's
//! samples (`TerrainSource::horizon_sample`) on a few nested grids, each twice as coarse as the one inside
//! it (like a geometry clipmap), around a centre that follows the camera
//! in steps, on a thread of its own.
//!
//! Where chunks are loaded the horizon sinks under them (in the vertex
//! shaders, from the camera's live position), so the two never fight. The
//! water beyond the chunks (the sea, lakes) is a mesh of its own, drawn
//! like the chunks' water (see `build`), over the ground under it.

use std::sync::{
    mpsc::{self, Receiver, TryRecvError},
    Arc,
};

use super::{
    mesher::TerrainVertex,
    source::{RiverPiece, SurfaceBand, SurfaceSample, TerrainSource},
    water::WaterVertex,
    ChunkMesh,
};

/// Metres between the samples of the finest grid (a multiple of
/// `MACRO_STEP`, so samples fall on the large shape's grid).
pub const FINEST_STEP: f64 = 32.0;
/// Cells from the centre to the edge of each grid.
const HALF_CELLS: i32 = 64;
/// Grids, each twice as coarse and twice as wide as the one inside it.
pub const LEVELS: usize = 4;
/// The centre moves in steps of this many metres (twice the coarsest
/// grid's step, so every grid stays on its own lattice).
pub const CENTRE_STEP: f64 = FINEST_STEP * (1 << LEVELS) as f64;
/// Typical width of the sea and of lakes for the far water's waves (see
/// `WaterVertex::body_size`).
const SEA_SIZE: f32 = 1000.0;
const LAKE_SIZE: f32 = 300.0;
/// Ground up to this far above the water still gets far water over it, so
/// its edge lies under the shore.
const SHORE_MARGIN: f32 = 2.0;

/// How far under the ground's samples the horizon is drawn beyond the
/// chunks (see the terrain's vertex program), metres.
pub const SINK: f32 = 4.0;
/// Rivers on the horizon lie this far over the ground drawn, metres, and
/// this much more per metre between the grid's samples there: a band's
/// straight stretches would cut through the coarse grids' rises.
const RIVER_LIFT: f32 = 1.0;
const RIVER_LIFT_PER_STEP: f32 = 0.04;
/// Narrowest a river is drawn on the horizon, metres: narrower ones would
/// show in pieces from far away.
const MIN_RIVER_WIDTH: f32 = 6.0;
/// Longest stretch of a river band between two of its vertices, metres, so
/// it follows the ground's grid instead of cutting through its hills.
const RIVER_STEP: f64 = 12.0;
/// Strips of ground on the horizon (see `TerrainSource::far_bands`) lie this
/// far over its ground (before `SINK` and plus `RIVER_LIFT_PER_STEP` per
/// metre of grid step), and are drawn at least this wide.
const BAND_LIFT: f32 = 0.5;
const MIN_BAND_WIDTH: f32 = 6.0;
/// Depth the water of a river on the horizon is coloured for, metres.
const RIVER_DEPTH: f32 = 2.0;
/// Speed of the water of a river on the horizon, metres per second.
const RIVER_FLOW: f32 = 1.0;

/// What `build` makes: the ground, and the water over it.
pub struct Built {
    pub ground: (Vec<HorizonVertex>, Vec<u32>),
    pub water: (Vec<WaterVertex>, Vec<u32>),
    pub heights: Arc<HeightGrid>,
}

/// The heights of the horizon's ground as it is drawn (before it is sunk
/// by `SINK`), for what stands on it (see `far_scatter`).
pub struct HeightGrid {
    centre: [f64; 2],
    /// Per grid, from the finest: the heights of its vertices, row by row.
    levels: Vec<Vec<f32>>,
}

impl HeightGrid {
    /// Where the grids are centred (world x, z).
    pub fn centre(&self) -> [f64; 2] {
        self.centre
    }

    /// The height of the drawn ground at world `(x, z)`, on the finest grid
    /// that holds it (the one drawn there), on its triangles; `None` beyond
    /// the horizon.
    pub fn height(&self, x: f64, z: f64) -> Option<f32> {
        self.sample(x, z).map(|(height, _)| height)
    }

    /// `height`, and the step of the grid it is from (metres).
    pub fn sample(&self, x: f64, z: f64) -> Option<(f32, f32)> {
        let n = (2 * HALF_CELLS + 1) as usize;
        for (level, heights) in self.levels.iter().enumerate() {
            let step = FINEST_STEP * (1 << level) as f64;
            let fx = (x - self.centre[0]) / step + HALF_CELLS as f64;
            let fz = (z - self.centre[1]) / step + HALF_CELLS as f64;
            let last = (n - 1) as f64;
            if !(0.0..last).contains(&fx) || !(0.0..last).contains(&fz) {
                continue;
            }
            let (i, j) = (fx as usize, fz as usize);
            let (u, v) = ((fx - i as f64) as f32, (fz - j as f64) as f32);
            let h = |i: usize, j: usize| heights[j * n + i];
            // The triangles of `build`: split from (i + 1, j) to (i, j + 1).
            let (a, b, c, d) = (h(i, j), h(i + 1, j), h(i, j + 1), h(i + 1, j + 1));
            let height = if u + v <= 1.0 {
                a + u * (b - a) + v * (c - a)
            } else {
                d + (1.0 - u) * (c - d) + (1.0 - v) * (b - d)
            };
            return Some((height, step as f32));
        }
        None
    }
}

/// The horizon's vertices: the chunks', so the terrain's programs draw it.
pub type HorizonVertex = TerrainVertex;

/// Distance in metres from the centre to the horizon's edge.
pub fn reach() -> f64 {
    FINEST_STEP * HALF_CELLS as f64 * (1 << (LEVELS - 1)) as f64
}

/// The horizon around `centre` (world `(x, z)`, a multiple of
/// `CENTRE_STEP`): the ground's vertices in world metres and triangles
/// facing up, and the water's (the sea and lakes, flat at their levels)
/// over the cells where the ground comes near them.
pub fn build(source: &dyn TerrainSource, centre: [f64; 2]) -> Built {
    let sea = source.sea_level();
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut water_vertices = Vec::new();
    let mut water_indices = Vec::new();
    let mut grid = HeightGrid {
        centre,
        levels: Vec::with_capacity(LEVELS),
    };
    for level in 0..LEVELS {
        let step = FINEST_STEP * (1 << level) as f64;
        let n = 2 * HALF_CELLS + 1;
        // Heights one sample beyond the grid, for the normals.
        let side = n + 2;
        let at = |i: i32| (i - 1 - HALF_CELLS) as f64 * step;
        let mut samples = Vec::with_capacity((side * side) as usize);
        for j in 0..side {
            for i in 0..side {
                let sample = source.horizon_sample(centre[0] + at(i), centre[1] + at(j));
                samples.push((sample.height, sample.surface, sample.water));
            }
        }
        // The ground (under the water too: the water is drawn over it).
        let height = |i: i32, j: i32| samples[((j + 1) * side + i + 1) as usize].0;
        // On the edge a coarser grid meets, every other vertex lies on the
        // coarse grid's edge between its neighbours: no cracks.
        let outer = level + 1 < LEVELS;
        let edge_height = |i: i32, j: i32| {
            let on_edge_x = i == 0 || i == n - 1;
            let on_edge_z = j == 0 || j == n - 1;
            if outer && on_edge_z && i % 2 == 1 {
                0.5 * (height(i - 1, j) + height(i + 1, j))
            } else if outer && on_edge_x && j % 2 == 1 {
                0.5 * (height(i, j - 1) + height(i, j + 1))
            } else {
                height(i, j)
            }
        };
        let first = vertices.len() as u32;
        let mut drawn = Vec::with_capacity((n * n) as usize);
        for j in 0..n {
            for i in 0..n {
                let (_, surface, _) = samples[((j + 1) * side + i + 1) as usize];
                let y = edge_height(i, j);
                drawn.push(y);
                let dx = height(i + 1, j) - height(i - 1, j);
                let dz = height(i, j + 1) - height(i, j - 1);
                let normal = [-dx, 2.0 * step as f32, -dz];
                let length = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
                vertices.push(HorizonVertex {
                    position: [(centre[0] + at(i + 1)) as f32, y, (centre[1] + at(j + 1)) as f32],
                    normal: normal.map(|c| c / length),
                    color: surface.color,
                    data: surface.data,
                });
            }
        }
        // The inner grid covers the middle of every grid but the finest.
        let hole = if level == 0 { 0 } else { HALF_CELLS / 2 };
        let covered = |i: i32, j: i32| {
            let (ci, cj) = (i - HALF_CELLS, j - HALF_CELLS);
            level > 0 && ci >= -hole && ci < hole && cj >= -hole && cj < hole
        };
        // The water over this grid: cells with a corner near or under it,
        // flat at its level (a dry corner takes its wet neighbours' level).
        let sample = |i: i32, j: i32| samples[((j + 1) * side + i + 1) as usize];
        let wet = |i: i32, j: i32| {
            let (h, _, water) = sample(i, j);
            h < water + SHORE_MARGIN
        };
        let surface = |i: i32, j: i32| {
            let (_, _, own) = sample(i, j);
            if wet(i, j) {
                return own;
            }
            let mut best: Option<f32> = None;
            for dj in -1..=1 {
                for di in -1..=1 {
                    if wet(i + di, j + dj) {
                        let l = sample(i + di, j + dj).2;
                        best = Some(best.map_or(l, |b: f32| b.max(l)));
                    }
                }
            }
            best.unwrap_or(own)
        };
        let mut index_of = vec![u32::MAX; (n * n) as usize];
        for j in 0..n - 1 {
            for i in 0..n - 1 {
                if covered(i, j) || !(0..2).any(|dj| (0..2).any(|di| wet(i + di, j + dj))) {
                    continue;
                }
                let mut corner = |i: i32, j: i32| {
                    let slot = &mut index_of[(j * n + i) as usize];
                    if *slot == u32::MAX {
                        let (h, _, _) = sample(i, j);
                        let level = surface(i, j);
                        *slot = water_vertices.len() as u32;
                        water_vertices.push(WaterVertex {
                            position: [(centre[0] + at(i + 1)) as f32, level, (centre[1] + at(j + 1)) as f32],
                            depth: level - h,
                            body_size: if sea.is_some_and(|sea| level <= sea + 0.01) { SEA_SIZE } else { LAKE_SIZE },
                            flow: [0.0; 2],
                        });
                    }
                    *slot
                };
                let (a, b, c, d) = (corner(i, j), corner(i + 1, j), corner(i, j + 1), corner(i + 1, j + 1));
                water_indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
        for j in 0..n - 1 {
            for i in 0..n - 1 {
                if covered(i, j) {
                    continue;
                }
                let v = |i: i32, j: i32| first + (j * n + i) as u32;
                let (a, b, c, d) = (v(i, j), v(i + 1, j), v(i, j + 1), v(i + 1, j + 1));
                // Up-facing, as the chunks' triangles.
                indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
        grid.levels.push(drawn);
    }
    river_bands(&source.far_rivers(centre, reach()), &grid, &mut water_vertices, &mut water_indices);
    surface_bands(&source.far_bands(centre, reach()), &grid, &mut vertices, &mut indices);
    Built {
        ground: (vertices, indices),
        water: (water_vertices, water_indices),
        heights: Arc::new(grid),
    }
}

/// The rivers `pieces` as bands of water lying on the horizon's ground
/// (`grid`), added to `vertices` and `indices`. Their `body_size` is
/// negative: the water's vertex program sinks them under the chunks
/// further than the rest of the far water (see `water_vertex.glsl`).
fn river_bands(pieces: &[RiverPiece], grid: &HeightGrid, vertices: &mut Vec<WaterVertex>, indices: &mut Vec<u32>) {
    for chain in chains(pieces) {
        // Points along the chain, no further apart than `RIVER_STEP`.
        let mut points: Vec<([f64; 2], f32)> = Vec::new();
        for piece in &chain {
            let length = (piece.b[0] - piece.a[0]).hypot(piece.b[1] - piece.a[1]);
            let steps = (length / RIVER_STEP).ceil().max(1.0) as usize;
            let first = if points.is_empty() { 0 } else { 1 };
            for k in first..=steps {
                let t = k as f64 / steps as f64;
                let p = [piece.a[0] + (piece.b[0] - piece.a[0]) * t, piece.a[1] + (piece.b[1] - piece.a[1]) * t];
                let width = piece.width[0] + (piece.width[1] - piece.width[0]) * t as f32;
                points.push((p, width.max(MIN_RIVER_WIDTH)));
            }
        }
        if points.len() < 2 {
            continue;
        }
        let first = vertices.len() as u32;
        let mut made = 0u32;
        for k in 0..points.len() {
            let (prev, next) = (points[k.saturating_sub(1)].0, points[(k + 1).min(points.len() - 1)].0);
            let (dx, dz) = (next[0] - prev[0], next[1] - prev[1]);
            let length = dx.hypot(dz).max(1e-9);
            let (dx, dz) = (dx / length, dz / length);
            let (p, width) = points[k];
            let half = width as f64 * 0.5;
            let side = |s: f64| {
                let (x, z) = (p[0] - dz * half * s, p[1] + dx * half * s);
                let (ground, step) = grid.sample(x, z)?;
                Some(WaterVertex {
                    position: [x as f32, ground - SINK + RIVER_LIFT + RIVER_LIFT_PER_STEP * step, z as f32],
                    depth: RIVER_DEPTH,
                    body_size: -width,
                    flow: [dx as f32 * RIVER_FLOW, dz as f32 * RIVER_FLOW],
                })
            };
            let (Some(left), Some(right)) = (side(-1.0), side(1.0)) else {
                // Beyond the horizon: the band ends here.
                break;
            };
            vertices.push(left);
            vertices.push(right);
            made += 1;
        }
        for k in 0..made.saturating_sub(1) {
            let (a, b) = (first + 2 * k, first + 2 * k + 1);
            let (c, d) = (a + 2, b + 2);
            // Up-facing whichever way the river runs.
            let (pa, pb, pc) = (vertices[a as usize].position, vertices[b as usize].position, vertices[c as usize].position);
            let cross = (pb[2] - pa[2]) * (pc[0] - pa[0]) - (pb[0] - pa[0]) * (pc[2] - pa[2]);
            if cross > 0.0 {
                indices.extend_from_slice(&[a, b, c, b, d, c]);
            } else {
                indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
    }
}

/// The strips of ground `bands` laid on the horizon's ground (`grid`), a
/// little over it, added to its `vertices` and `indices`.
fn surface_bands(bands: &[SurfaceBand], grid: &HeightGrid, vertices: &mut Vec<HorizonVertex>, indices: &mut Vec<u32>) {
    let mix = |a: &SurfaceSample, b: &SurfaceSample, t: f32| SurfaceSample {
        color: std::array::from_fn(|i| a.color[i] + (b.color[i] - a.color[i]) * t),
        data: std::array::from_fn(|i| a.data[i] + (b.data[i] - a.data[i]) * t),
    };
    for band in bands {
        // Points along the band, no further apart than `RIVER_STEP`.
        let mut points: Vec<([f64; 2], f32, SurfaceSample)> = Vec::new();
        for pair in band.points.windows(2) {
            let ((a, wa, sa), (b, wb, sb)) = (&pair[0], &pair[1]);
            let length = (b[0] - a[0]).hypot(b[1] - a[1]);
            let steps = (length / RIVER_STEP).ceil().max(1.0) as usize;
            let first = if points.is_empty() { 0 } else { 1 };
            for k in first..=steps {
                let t = k as f64 / steps as f64;
                let p = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
                let width = (wa + (wb - wa) * t as f32).max(MIN_BAND_WIDTH);
                points.push((p, width, mix(sa, sb, t as f32)));
            }
        }
        if points.len() < 2 {
            continue;
        }
        let first = vertices.len() as u32;
        let mut made = 0u32;
        for k in 0..points.len() {
            let (prev, next) = (points[k.saturating_sub(1)].0, points[(k + 1).min(points.len() - 1)].0);
            let (dx, dz) = (next[0] - prev[0], next[1] - prev[1]);
            let length = dx.hypot(dz).max(1e-9);
            let (dx, dz) = (dx / length, dz / length);
            let (p, width, surface) = points[k];
            let half = width as f64 * 0.5;
            let side = |s: f64| {
                let (x, z) = (p[0] - dz * half * s, p[1] + dx * half * s);
                let (ground, step) = grid.sample(x, z)?;
                // The ground's slope there, a grid step to either side.
                let d = step as f64;
                let h = |x: f64, z: f64| grid.height(x, z).unwrap_or(ground);
                let normal = [h(x - d, z) - h(x + d, z), 2.0 * step, h(x, z - d) - h(x, z + d)];
                let length = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
                Some(HorizonVertex {
                    position: [x as f32, ground + BAND_LIFT + RIVER_LIFT_PER_STEP * step, z as f32],
                    normal: normal.map(|c| c / length),
                    color: surface.color,
                    data: surface.data,
                })
            };
            let (Some(left), Some(right)) = (side(-1.0), side(1.0)) else {
                break;
            };
            vertices.push(left);
            vertices.push(right);
            made += 1;
        }
        for k in 0..made.saturating_sub(1) {
            let (a, b) = (first + 2 * k, first + 2 * k + 1);
            let (c, d) = (a + 2, b + 2);
            let (pa, pb, pc) = (vertices[a as usize].position, vertices[b as usize].position, vertices[c as usize].position);
            let cross = (pb[2] - pa[2]) * (pc[0] - pa[0]) - (pb[0] - pa[0]) * (pc[2] - pa[2]);
            if cross > 0.0 {
                indices.extend_from_slice(&[a, b, c, b, d, c]);
            } else {
                indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
    }
}

/// `pieces` joined into runs: each piece followed by the one beginning
/// where it ends (exactly).
fn chains(pieces: &[RiverPiece]) -> Vec<Vec<RiverPiece>> {
    use std::collections::HashMap;
    let key = |p: [f64; 2]| (p[0].to_bits(), p[1].to_bits());
    let mut starting: HashMap<(u64, u64), usize> = HashMap::new();
    for (i, piece) in pieces.iter().enumerate() {
        starting.entry(key(piece.a)).or_insert(i);
    }
    let ending: std::collections::HashSet<(u64, u64)> = pieces.iter().map(|p| key(p.b)).collect();
    let mut used = vec![false; pieces.len()];
    let mut out = Vec::new();
    // Runs from where no piece leads in first, then whatever is left
    // (loops).
    let starts: Vec<usize> = (0..pieces.len())
        .filter(|&i| !ending.contains(&key(pieces[i].a)))
        .chain(0..pieces.len())
        .collect();
    for start in starts {
        let mut chain = Vec::new();
        let mut at = Some(start);
        while let Some(i) = at {
            if used[i] {
                break;
            }
            used[i] = true;
            chain.push(pieces[i]);
            at = starting.get(&key(pieces[i].b)).copied();
        }
        if !chain.is_empty() {
            out.push(chain);
        }
    }
    out
}

/// The horizon the terrain draws, made again on a thread of its own when
/// the camera has moved a step.
pub struct Horizon {
    mesh: Option<ChunkMesh<HorizonVertex>>,
    /// The heights of the ground drawn.
    heights: Option<Arc<HeightGrid>>,
    /// The water beyond the chunks.
    water: Option<ChunkMesh<WaterVertex>>,
    /// Centre of the mesh drawn, and of the one being made.
    centre: Option<[f64; 2]>,
    pending: Option<([f64; 2], Receiver<Built>)>,
}

impl Horizon {
    pub fn new() -> Self {
        Self {
            mesh: None,
            heights: None,
            water: None,
            centre: None,
            pending: None,
        }
    }

    /// Forget the mesh (the world changed).
    pub fn invalidate(&mut self) {
        self.mesh = None;
        self.heights = None;
        self.water = None;
        self.centre = None;
        self.pending = None;
    }

    /// Make it again where it is (the world finished something it shows,
    /// see `TerrainSource::status`), drawing the one there is meanwhile.
    pub fn refresh(&mut self) {
        self.centre = None;
        self.pending = None;
    }

    /// Follow the camera at world `(x, z)`.
    pub fn update(&mut self, source: &Arc<dyn TerrainSource>, x: f64, z: f64) {
        if let Some((centre, rx)) = &self.pending {
            let received = rx.try_recv();
            if let Err(TryRecvError::Disconnected) = received {
                // The build panicked (reported by the panic hook): keep
                // drawing what there is, and try again from the next
                // centre.
                log::error!("the terrain's horizon could not be made at {centre:?}");
                self.centre = Some(*centre);
                self.pending = None;
            } else if let Ok(built) = received {
                let (vertices, indices) = built.ground;
                let mut mesh = ChunkMesh::new(vertices, Some(indices));
                mesh.buffer_and_release();
                self.mesh = Some(mesh);
                self.heights = Some(built.heights);
                let (vertices, indices) = built.water;
                self.water = (!indices.is_empty()).then(|| {
                    let mut water = ChunkMesh::new(vertices, Some(indices));
                    water.buffer_and_release();
                    water
                });
                self.centre = Some(*centre);
                self.pending = None;
            }
        }
        let snap = |v: f64| (v / CENTRE_STEP).round() * CENTRE_STEP;
        let wanted = [snap(x), snap(z)];
        let current = self.pending.as_ref().map(|(c, _)| *c).or(self.centre);
        if current != Some(wanted) {
            let (tx, rx) = mpsc::channel();
            let source = source.clone();
            std::thread::spawn(move || {
                let _ = tx.send(build(&*source, wanted));
            });
            self.pending = Some((wanted, rx));
        }
    }

    pub fn mesh(&self) -> Option<&ChunkMesh<HorizonVertex>> {
        self.mesh.as_ref()
    }

    /// The heights of the ground drawn, once there is a mesh.
    pub fn heights(&self) -> Option<&Arc<HeightGrid>> {
        self.heights.as_ref()
    }

    /// The water beyond the chunks, if there is any in reach.
    pub fn water(&self) -> Option<&ChunkMesh<WaterVertex>> {
        self.water.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grids_meet_without_cracks_and_face_up() {
        use crate::terrain::{hills::{Hills, HillsSettings}, source::TerrainGenerator};
        let source = Hills::create(1, HillsSettings::default());
        let (vertices, indices) = build(&*source, [-2048.0, -6144.0]).ground;
        assert_eq!(vertices.len(), LEVELS * (2 * HALF_CELLS as usize + 1).pow(2));
        for t in indices.chunks(3) {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| vertices[i as usize].position);
            let e1 = [b[0] - a[0], b[2] - a[2]];
            let e2 = [c[0] - a[0], c[2] - a[2]];
            assert!(e1[1] * e2[0] - e1[0] * e2[1] > 0.0, "triangle facing down");
        }
        // Where a grid meets the coarser one around it, its edge vertices
        // lie on the coarser grid's edges: a vertex halfway between two of
        // the coarse grid has the mean of their heights.
        let n = 2 * HALF_CELLS as usize + 1;
        let fine = &vertices[..n * n];
        let coarse = &vertices[n * n..2 * n * n];
        let find = |grid: &[HorizonVertex], x: f32, z: f32| {
            grid.iter().find(|v| v.position[0] == x && v.position[2] == z).map(|v| v.position[1])
        };
        for i in (0..n - 2).step_by(2) {
            let (a, mid, b) = (fine[i].position, fine[i + 1].position, fine[i + 2].position);
            let (ca, cb) = (find(coarse, a[0], a[2]).unwrap(), find(coarse, b[0], b[2]).unwrap());
            assert!((mid[1] - 0.5 * (ca + cb)).abs() < 1e-3, "crack at {mid:?}");
        }
        assert!(reach() >= 16000.0);
    }
}
