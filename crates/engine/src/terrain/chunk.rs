//! One chunk of the terrain: its ground at every level of detail (see
//! `mesher`), its water (see `water`) and the spots scattered models stand
//! on, made on a loader thread from the terrain's source.

use std::cell::Cell;

use cgmath::{Matrix4, Point3, Vector3};
use glfw::{Glfw, WindowEvent};

use crate::core::{
    entity::{component::Component, Entity},
    renderer::shader::Shader,
    scene::Scene,
};

use super::{
    mesher::{self, TerrainVertex, LODS},
    source::{ChunkCoord, TerrainSource},
    water::{self, WaterVertex},
    ChunkBounds, ChunkMesh, CHUNK_SIZE, CHUNK_SIZE_FLOAT,
};

/// Up to which distance from the camera (to the nearest point of a chunk,
/// in metres) each level but the last is drawn.
const LOD_DISTANCES: [f32; LODS - 1] = [160.0, 320.0, 560.0];

pub struct TerrainChunk {
    coord: ChunkCoord,
    /// The ground at each level of detail, finest first (see
    /// `mesher::mesh_chunk`); empty for chunks above the ground.
    meshes: Vec<ChunkMesh<TerrainVertex>>,
    /// Indices of the finest mesh that are ground; its skirt follows.
    ground_indices: usize,
    /// World box around every level's mesh, skirts and all.
    drawn_bounds: ChunkBounds,
    /// The water's surface over the chunk's square, made by the lowest
    /// chunk of its column. It reaches under the shore (see `water`).
    water_mesh: Option<ChunkMesh<WaterVertex>>,
    /// The level it was last drawn at (see `note_level`).
    drawn_level: Cell<Option<usize>>,
}

impl TerrainChunk {
    /// The chunk at `coord` of the world `source`, in full `detail` (with
    /// caves and the like) or not.
    pub fn generate(source: &dyn TerrainSource, coord: ChunkCoord, detail: bool) -> Self {
        Self::generate_for(source, coord, detail, true)
    }

    /// `generate`, or, unless `drawn`, only what collides (where nothing
    /// is drawn: a server without a GPU): the finest level's ground,
    /// unpainted and without its skirt, and no water.
    pub fn generate_for(source: &dyn TerrainSource, coord: ChunkCoord, detail: bool, drawn: bool) -> Self {
        let surface = source.chunk(coord, detail);
        let meshed = if drawn {
            mesher::mesh_chunk(&*surface)
        } else {
            mesher::mesh_collider(&*surface)
        };
        let lowest = (surface.field().min_height / CHUNK_SIZE_FLOAT).floor() as i32;
        drop(surface);
        let mut chunk = Self {
            coord,
            meshes: meshed
                .levels
                .into_iter()
                .map(|(vertices, indices)| ChunkMesh::new(vertices, Some(indices)))
                .collect(),
            ground_indices: meshed.ground_indices,
            drawn_bounds: ChunkBounds {
                min: (0, 0, 0),
                max: (0, 0, 0),
            },
            water_mesh: None,
            drawn_level: Cell::new(None),
        };
        // The lowest chunk of the column (the one holding its lowest
        // ground) has the water over it, whatever the water's height, so
        // exactly one chunk of a column has it.
        if drawn && coord.y == lowest {
            chunk.water_mesh = chunk.generate_water_mesh(source);
        }
        chunk.drawn_bounds = chunk.measure_drawn_bounds();
        chunk
    }

    pub fn coord(&self) -> ChunkCoord {
        self.coord
    }

    /// The chunk's corner, in world metres.
    pub fn position(&self) -> Point3<f32> {
        let [x, y, z] = self.coord.origin();
        Point3::new(x as f32, y as f32, z as f32)
    }

    /// The column of chunks (x, z) it belongs to.
    pub fn column(&self) -> (i32, i32) {
        (self.coord.x, self.coord.z)
    }

    /// Whether the chunk turned out to hold nothing to draw or collide
    /// with; such chunks are dropped right away.
    pub fn is_empty(&self) -> bool {
        self.meshes.is_empty() && self.water_mesh.is_none()
    }

    /// The water surface over this chunk's square (see `water`).
    fn generate_water_mesh(&self, source: &dyn TerrainSource) -> Option<ChunkMesh<WaterVertex>> {
        let [x0, y0, z0] = self.coord.origin();
        let (vertices, indices) = water::generate(
            |x, z| source.surface_height(x as f64, z as f64),
            |x, z| source.water_level(x as f64, z as f64),
            |x, z| source.water_flow(x as f64, z as f64),
            x0 as f32,
            z0 as f32,
            CHUNK_SIZE_FLOAT,
            y0 as f32,
        )?;
        Some(ChunkMesh::new(vertices, Some(indices)))
    }

    /// The box around every vertex of every level (see `drawn_bounds`),
    /// rounded outwards to whole metres.
    fn measure_drawn_bounds(&self) -> ChunkBounds {
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        let mut add = |p: &[f32; 3]| {
            for i in 0..3 {
                min[i] = min[i].min(p[i]);
                max[i] = max[i].max(p[i]);
            }
        };
        for mesh in &self.meshes {
            mesh.vertices.iter().for_each(|v| add(&v.position));
        }
        if let Some(water) = &self.water_mesh {
            // Waves rise and fall a little around the surface.
            for v in &water.vertices {
                add(&[v.position[0], v.position[1] - 2.0, v.position[2]]);
                add(&[v.position[0], v.position[1] + 2.0, v.position[2]]);
            }
        }
        if min[0] > max[0] {
            return self.get_bounds();
        }
        let origin = self.coord.origin().map(|c| c as f32);
        let low = |i: usize| (origin[i] + min[i]).floor() as i32;
        let high = |i: usize| (origin[i] + max[i]).ceil() as i32;
        ChunkBounds {
            min: (low(0), low(1), low(2)),
            max: (high(0), high(1), high(2)),
        }
    }

    /// The level of detail to draw at `distance` metres from the camera.
    fn lod_at(distance: f32) -> usize {
        // The graphics settings move the distances out or in.
        let distance = distance / crate::core::graphics::engine().terrain_lod_factor;
        LOD_DISTANCES
            .iter()
            .position(|&limit| distance < limit)
            .unwrap_or(LODS - 1)
    }

    /// Spots on this chunk where models of a scatter layer may stand, as
    /// world-space `(x, y, z, yaw_radians)`: `attempts` random spots of the
    /// chunk's square, of those the ones whose ground lies in the chunk and
    /// as many as the source lets grow there (`scatter_chance`, for the
    /// layer's `tag`). `salt` varies the pattern so each model gets its
    /// own; the same chunk, salt and attempts always give the same spots.
    pub fn scatter_points(
        &self,
        source: &dyn TerrainSource,
        tag: &str,
        salt: u64,
        attempts: u32,
    ) -> Vec<(f32, f32, f32, f32)> {
        if self.meshes.is_empty() {
            return Vec::new();
        }
        scatter_points_at(source, self.coord, tag, salt, attempts)
    }

    /// Whether the chunk holds any ground (models stand only on that).
    pub fn has_ground(&self) -> bool {
        !self.meshes.is_empty()
    }

    pub fn get_bounds(&self) -> ChunkBounds {
        let [x, y, z] = [self.coord.x, self.coord.y, self.coord.z].map(|c| c * CHUNK_SIZE as i32);
        let size = CHUNK_SIZE as i32;
        ChunkBounds {
            min: (x, y, z),
            max: (x + size, y + size, z + size),
        }
    }

    /// A box around everything the chunk draws, for culling: far flatter
    /// than the chunk's cube.
    pub fn get_drawn_bounds(&self) -> ChunkBounds {
        self.drawn_bounds
    }

    /// Buffer the meshes and drop their copies in memory. The collider and
    /// the drawn bounds are made before this.
    pub fn buffer_data(&mut self) {
        for mesh in &mut self.meshes {
            mesh.buffer_and_release();
        }
        if let Some(water_mesh) = &mut self.water_mesh {
            water_mesh.buffer_and_release();
        }
    }

    pub fn get_triangle_count(&self) -> usize {
        self.meshes.first().map_or(0, ChunkMesh::get_triangle_count)
    }

    /// The finest level's ground (without its skirt), for the collider:
    /// chunk-local vertices and triangles. Empty once the chunk is buffered.
    pub fn collision_mesh(&self) -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
        let Some(mesh) = self.meshes.first() else {
            return (Vec::new(), Vec::new());
        };
        let vertices = mesh.vertices.iter().map(|v| v.position).collect();
        let indices = mesh.indices.as_deref().unwrap_or_default();
        let ground = &indices[..self.ground_indices.min(indices.len())];
        (vertices, ground.chunks(3).map(|c| [c[0], c[1], c[2]]).collect())
    }

    /// The meshes as made, before buffering: every level's vertices and
    /// triangles (finest first), and how many indices of the finest are
    /// ground. For tests and tools.
    pub fn levels(&self) -> (Vec<(&[TerrainVertex], &[u32])>, usize) {
        let levels = self
            .meshes
            .iter()
            .map(|m| (m.vertices.as_slice(), m.indices.as_deref().unwrap_or_default()))
            .collect();
        (levels, self.ground_indices)
    }

    /// Note the level it is drawn at for `distance` metres from the camera;
    /// whether that differs from the one it was drawn at before (or it was
    /// not drawn yet): then shadows drawn before show another shape.
    pub fn note_level(&self, distance: f32) -> bool {
        let level = Self::lod_at(distance).min(self.meshes.len().saturating_sub(1));
        self.drawn_level.replace(Some(level)) != Some(level)
    }

    /// Draw the chunk's ground at the level for `distance` metres from the
    /// camera, with `shader` bound and set up by the terrain.
    pub fn draw(&self, shader: &Shader, transform: &Matrix4<f32>, distance: f32) {
        if let Some(mesh) = self.meshes.get(Self::lod_at(distance)).or(self.meshes.last()) {
            let corner = self.position();
            mesh.render(
                shader,
                &(transform * Matrix4::from_translation(Vector3::new(corner.x, corner.y, corner.z))),
                None,
            );
        }
    }

    /// Hand the mesh `draw` would draw to the ray traced shadows' scene.
    pub fn collect_for_ray_tracing(&self, transform: &Matrix4<f32>, distance: f32) {
        let Some(mesh) = self.meshes.get(Self::lod_at(distance)).or(self.meshes.last()) else {
            return;
        };
        let Some(geometry) = mesh.blas_geometry() else {
            return;
        };
        let corner = self.position();
        let placed = transform * Matrix4::from_translation(Vector3::new(corner.x, corner.y, corner.z));
        // The ground's colour follows position and normal (`TerrainVertex`).
        let look = crate::core::renderer::raytracing::Look::ground(
            std::mem::offset_of!(super::mesher::TerrainVertex, color) as u32,
        );
        crate::core::renderer::raytracing::add_mesh(geometry, None, &look, &placed);
    }

    /// The chunk's water and where it is drawn, under `parent_transform`
    /// (see `water::draw`).
    pub fn water(&self, parent_transform: &Matrix4<f32>) -> Option<(&ChunkMesh<WaterVertex>, Matrix4<f32>)> {
        let water_mesh = self.water_mesh.as_ref()?;
        let corner = self.position();
        let model = parent_transform * Matrix4::from_translation(Vector3::new(corner.x, corner.y, corner.z));
        Some((water_mesh, model))
    }
}

impl Component for TerrainChunk {
    fn update(&mut self, _: &mut Scene, _: &mut Entity, _: f64) {}

    fn render(&self, _: &Scene, _: &Entity, _: &Matrix4<f32>, _: &Matrix4<f32>) {
        // Drawn by the terrain, with the other chunks (see `draw`).
    }

    fn handle_event(&mut self, _: &mut Glfw, _: &mut glfw::Window, _: &WindowEvent) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::{
        hills::{Hills, HillsSettings},
        mesher::DensityField,
        source::TerrainGenerator,
    };

    fn hills() -> std::sync::Arc<Hills> {
        Hills::create(2, HillsSettings::default())
    }

    #[test]
    fn levels_get_coarser_with_distance() {
        assert_eq!(TerrainChunk::lod_at(0.0), 0);
        assert_eq!(TerrainChunk::lod_at(200.0), 1);
        assert_eq!(TerrainChunk::lod_at(400.0), 2);
        assert_eq!(TerrainChunk::lod_at(5000.0), LODS - 1);
    }

    /// A chunk of the hills that has ground in it.
    fn ground_chunk(source: &Hills, x: i32, z: i32) -> ChunkCoord {
        let h = source.surface_height(x as f64 * 128.0 + 64.0, z as f64 * 128.0 + 64.0);
        ChunkCoord::new(x, (h / CHUNK_SIZE_FLOAT).floor() as i32, z)
    }

    #[test]
    fn neighbours_meet_at_their_edge_on_every_level() {
        let source = hills();
        let left = ground_chunk(&source, 0, 0);
        let right = ChunkCoord::new(1, left.y, 0);
        let n = CHUNK_SIZE_FLOAT;
        let (sl, sr) = (source.chunk(left, true), source.chunk(right, true));
        let (fl, fr) = (DensityField { field: sl.field() }, DensityField { field: sr.field() });
        let ground = |field: &DensityField, surface: &dyn crate::terrain::source::ChunkSurface, lod: usize, nets: bool| -> Vec<[f32; 3]> {
            let mesh = if nets {
                field.surface_nets(1 << lod, surface)
            } else {
                field.dual_contouring(1 << lod, surface)
            };
            mesh.unwrap().0.iter().map(|v| v.position).collect()
        };
        for (lod, nets) in (0..LODS).flat_map(|lod| [(lod, false), (lod, true)]) {
            let step = (1 << lod) as f32;
            let theirs = ground(&fr, &*sr, lod, nets);
            // The left chunk's last row of vertices lies in the right
            // one's first cells: the same points, so the meshes join.
            let shared: Vec<_> = ground(&fl, &*sl, lod, nets)
                .into_iter()
                .filter(|p| p[0] > n && p[0] < n + step && p[1] > 0.0 && p[1] < n)
                .collect();
            assert!(!shared.is_empty(), "level {lod}");
            for p in shared {
                assert!(
                    theirs.iter().any(|q| (q[0] + n - p[0]).abs() < 1e-3
                        && (q[1] - p[1]).abs() < 1e-3
                        && (q[2] - p[2]).abs() < 1e-3),
                    "level {lod}: {p:?}"
                );
            }
        }
    }

    #[test]
    fn the_finest_level_follows_the_ground() {
        let source = hills();
        let coord = ground_chunk(&source, 0, 0);
        let chunk = TerrainChunk::generate(&*source, coord, true);
        let (levels, ground_indices) = chunk.levels();
        let (vertices, indices) = levels[0];
        let origin = coord.origin();
        for &i in &indices[..ground_indices] {
            let v = vertices[i as usize];
            let [x, y, z] = v.position;
            let h = source.surface_height(origin[0] + x as f64, origin[2] + z as f64);
            assert!((origin[1] as f32 + y - h).abs() < 0.75, "{:?}", v.position);
            let length = v.normal.iter().map(|c| c * c).sum::<f32>().sqrt();
            assert!((length - 1.0).abs() < 1e-3 && v.normal[1] > 0.0);
        }
        // Triangles face out of the ground, as the normals do.
        for t in indices[..ground_indices].chunks(3) {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| vertices[i as usize]);
            let e1: [f32; 3] = std::array::from_fn(|k| b.position[k] - a.position[k]);
            let e2: [f32; 3] = std::array::from_fn(|k| c.position[k] - a.position[k]);
            let face_y = e1[2] * e2[0] - e1[0] * e2[2];
            assert!(face_y >= -1e-4, "triangle facing down");
        }
        // The collider is the ground alone.
        assert_eq!(chunk.collision_mesh().1.len() * 3, ground_indices);
        assert!(indices.len() > ground_indices, "no skirt");
    }

    #[test]
    fn the_drawn_bounds_hold_every_level_and_are_flatter_than_the_chunk() {
        let source = hills();
        let coord = ground_chunk(&source, 1, -1);
        let chunk = TerrainChunk::generate(&*source, coord, true);
        let b = chunk.get_drawn_bounds();
        let origin = coord.origin().map(|c| c as f32);
        for (vertices, _) in chunk.levels().0 {
            for v in vertices {
                let p: [f32; 3] = std::array::from_fn(|i| v.position[i] + origin[i]);
                assert!(p[0] >= b.min.0 as f32 && p[0] <= b.max.0 as f32);
                assert!(p[1] >= b.min.1 as f32 && p[1] <= b.max.1 as f32);
                assert!(p[2] >= b.min.2 as f32 && p[2] <= b.max.2 as f32);
            }
        }
        assert!(b.max.1 - b.min.1 < CHUNK_SIZE as i32, "{b:?}");
    }

    #[test]
    fn a_chunk_not_drawn_collides_as_a_drawn_one_does() {
        let source = hills();
        let coord = ground_chunk(&source, 0, 0);
        let drawn = TerrainChunk::generate(&*source, coord, true);
        let bare = TerrainChunk::generate_for(&*source, coord, true, false);
        let (dv, di) = drawn.collision_mesh();
        let (bv, bi) = bare.collision_mesh();
        assert!(!di.is_empty());
        // The drawn one's skirt follows its ground's vertices (no triangle
        // of the collider uses those).
        assert_eq!(dv[..bv.len()], bv[..]);
        assert_eq!(di, bi);
        // Nothing but that.
        assert_eq!(bare.levels().0.len(), 1);
        assert!(bare.water_mesh.is_none());
    }

    #[test]
    fn a_chunk_without_ground_has_no_meshes() {
        let source = hills();
        let chunk = TerrainChunk::generate(&*source, ChunkCoord::new(0, 8, 0), true);
        assert!(chunk.levels().0.is_empty());
    }

    #[test]
    fn scattered_spots_stand_on_the_ground_in_the_chunk() {
        let source = hills();
        let coord = ground_chunk(&source, 0, 0);
        let chunk = TerrainChunk::generate(&*source, coord, true);
        let spots = chunk.scatter_points(&*source, "", 7, 200);
        assert_eq!(spots, chunk.scatter_points(&*source, "", 7, 200));
        for (x, y, z, _) in spots {
            assert_eq!(y, source.surface_height(x as f64, z as f64));
            assert!(ChunkCoord::containing(x as f64, y as f64, z as f64) == coord);
        }
    }
}

/// Where a scatter layer (`tag`, `salt`, `attempts`) grows models in the
/// chunk at `coord` with ground: world position and turn. Safe to run off
/// the main thread (see `terrain::planting`).
pub fn scatter_points_at(
    source: &dyn TerrainSource,
    coord: ChunkCoord,
    tag: &str,
    salt: u64,
    attempts: u32,
) -> Vec<(f32, f32, f32, f32)> {
    let floor = coord.floor();
    let ceiling = floor + CHUNK_SIZE_FLOAT;
    let mut out = Vec::new();
    for (x, z, grow, yaw) in scatter_spots((coord.x, coord.z), salt, attempts) {
        let y = source.surface_height(x as f64, z as f64);
        if y < floor || y >= ceiling {
            continue;
        }
        if grow >= source.scatter_chance(tag, x as f64, y, z as f64) {
            continue;
        }
        out.push((x, y, z, yaw));
    }
    out
}

/// The spots a scatter layer with `salt` tries on the chunk square
/// `column` (`attempts` of them): world `(x, z)`, a number 0..1 that grows
/// a model there if it is under the chance there (see
/// `TerrainSource::scatter_chance`) and the model's turn. The same for
/// every chunk of the column, every time, near and far (see `far_scatter`).
pub fn scatter_spots(column: (i32, i32), salt: u64, attempts: u32) -> impl Iterator<Item = (f32, f32, f32, f32)> {
    let (x0, z0) = (column.0 as f32 * CHUNK_SIZE_FLOAT, column.1 as f32 * CHUNK_SIZE_FLOAT);
    let cx = column.0 as u32 as u64;
    let cz = column.1 as u32 as u64;
    let mut rng: u64 = ((cx << 32 | cz) ^ salt).wrapping_mul(0x9E3779B97F4A7C15);
    let mut next = move || {
        rng = rng
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (rng >> 32) as u32
    };
    let unit = |r: u32| (r >> 8) as f32 / ((1u32 << 24) as f32);
    (0..attempts).map(move |_| {
        let x = x0 + unit(next()) * CHUNK_SIZE_FLOAT;
        let z = z0 + unit(next()) * CHUNK_SIZE_FLOAT;
        // Drawn for every spot, so a spot's turn does not depend on
        // whether the ones before it grew.
        let (grow, yaw) = (unit(next()), unit(next()) * std::f32::consts::TAU);
        (x, z, grow, yaw)
    })
}
