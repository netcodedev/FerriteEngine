use std::time::Instant;

use cgmath::{Matrix4, Point3, SquareMatrix, Vector3};
use lazy_static::lazy_static;
use gl::types::GLuint;
use glfw::{Glfw, MouseButton, WindowEvent};
use std::sync::Arc;

use crate::{
    core::{
        entity::{component::Component, Entity},
        renderer::{
            depth_pass,
            hdr,
            line::Line,
            pbr::{self, Lighting},
            shader::{Shader, VertexAttributes},
            sky,
            texture::Texture,
        },
        scene::Scene,
    },
    terrain::{
        materials,
        scatter::{SCATTER_MAX_Y, SCATTER_MIN_Y},
        generator::{ChunkCoord, WorldGenerator},
        Chunk, ChunkBounds, CHUNK_SIZE, CHUNK_SIZE_FLOAT,
    },
};

thread_local! {
    static WATER_SHADER: Shader = Shader::new(
        &format!(
            "#version 460 core\n{}\n{}",
            include_str!("water_waves.glsl"),
            include_str!("water_vertex.glsl")
        ),
        &format!(
            "#version 460 core\n{}\n{}\n{}",
            pbr::LIGHTING_GLSL,
            include_str!("water_waves.glsl"),
            include_str!("water_fragment.glsl")
        ),
    );
}

lazy_static! {
    static ref START: Instant = Instant::now();
}

/// Uniforms `sky::CLOUD_FIELD_GLSL` needs that `pbr::LIGHTING_GLSL` does
/// not declare.
const CLOUD_UNIFORMS: &str = "uniform float uTime;\nuniform float cloudType;";

/// Texture units the opaque scene is bound to while the water draws (clear
/// of the terrain's textures, the clouds, ambient occlusion and shadows).
const SCENE_COLOR_UNIT: u32 = 11;
const SCENE_DEPTH_UNIT: u32 = 12;


use super::{
    density::{add_skirt, DensityField},
    water::{self, WaterVertex},
    ChunkMesh, DualContouringChunk, Vertex, LODS, WATER_LEVEL, LOD_DISTANCES, SKIRT_DEPTH, SKIRT_DEPTH_PER_STEP,
};

impl DualContouringChunk {
    /// The ground's meshes, finest first: surface nets over the density
    /// field (see `DensityField`) sampled 1 metre apart, then 2, 4 and 8.
    /// The field is a volume, so overhangs and caves mesh as well as hills.
    /// The normals of every level come from the full-detail field, so the
    /// light on the ground does not change when a chunk switches levels.
    /// Each mesh has a skirt along its open edges, pushed into the ground,
    /// deeper the coarser the level, which hides the gaps where a neighbour
    /// drawn at another level meets it. Also returns how many indices of
    /// the finest mesh are ground (before its skirt), for the collider.
    fn generate_meshes(&self) -> (Vec<ChunkMesh<Vertex>>, usize) {
        let coarsest = 1 << (LODS - 1);
        let field = DensityField::new(&self.generator, self.coord, coarsest);
        let mut ground_indices = 0;
        let mut meshes = Vec::with_capacity(LODS);
        for lod in 0..LODS {
            let step = 1 << lod;
            let Some((mut vertices, mut indices)) = field.surface_nets(step) else {
                // No ground in this chunk.
                return (Vec::new(), 0);
            };
            if lod == 0 {
                ground_indices = indices.len();
            }
            add_skirt(
                &mut vertices,
                &mut indices,
                SKIRT_DEPTH + SKIRT_DEPTH_PER_STEP * step as f32,
            );
            meshes.push(ChunkMesh::new(vertices, Some(indices)));
        }
        (meshes, ground_indices)
    }

    /// The box around every vertex of every level (see `drawn_bounds`),
    /// rounded outwards to whole metres.
    fn measure_drawn_bounds(&self) -> ChunkBounds {
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        for mesh in &self.meshes {
            for v in &mesh.vertices {
                for i in 0..3 {
                    min[i] = min[i].min(v.position[i]);
                    max[i] = max[i].max(v.position[i]);
                }
            }
        }
        if min[0] > max[0] {
            return self.get_bounds();
        }
        let origin = [self.position.0, self.position.1, self.position.2].map(|c| c * CHUNK_SIZE_FLOAT);
        let low = |i: usize| (origin[i] + min[i]).floor() as i32;
        let high = |i: usize| (origin[i] + max[i]).ceil() as i32;
        ChunkBounds {
            min: (low(0), low(1), low(2)),
            max: (high(0), high(1), high(2)),
        }
    }

    /// The level of detail to draw at `distance` metres from the camera.
    fn lod_at(distance: f32) -> usize {
        LOD_DISTANCES
            .iter()
            .position(|&limit| distance < limit)
            .unwrap_or(LODS - 1)
    }

    /// World height of the ground at world `(x, z)`: where the density
    /// field crosses zero (see `density`).
    pub(super) fn ground_height(&self, x: f32, z: f32) -> f32 {
        self.generator.surface_height(x as f64, z as f64)
    }

    /// The water surface over this chunk (see `water`), in the chunk the
    /// water level lies in.
    fn generate_water_mesh(&self) -> Option<ChunkMesh<WaterVertex>> {
        if self.coord.y != (WATER_LEVEL / CHUNK_SIZE_FLOAT).floor() as i32 {
            return None;
        }
        let (vertices, indices) = water::generate(
            |x, z| self.ground_height(x, z),
            self.position.0 * CHUNK_SIZE_FLOAT,
            self.position.2 * CHUNK_SIZE_FLOAT,
            CHUNK_SIZE_FLOAT,
        )?;
        Some(ChunkMesh::new(vertices, Some(indices)))
    }

    /// Scatter spots across this chunk's XZ footprint using the same noise
    /// that drives the chunk's terrain — so the height filter sees the exact
    /// surface a model would land on. Returns world-space `(x, y, z, yaw)`
    /// tuples. A chunk only gives the spots whose ground lies in it.
    fn scatter(&self, salt: u64, attempts: u32) -> Vec<(f32, f32, f32, f32)> {
        let floor = self.coord.y as f32 * CHUNK_SIZE_FLOAT;
        let ceiling = floor + CHUNK_SIZE_FLOAT;
        if self.meshes.is_empty() {
            return Vec::new();
        }
        let x0 = self.position.0 * CHUNK_SIZE_FLOAT;
        let z0 = self.position.2 * CHUNK_SIZE_FLOAT;

        // Deterministic chunk-local RNG so the same chunk always scatters
        // the same spots for a given salt (across runs, across chunk
        // reloads).
        let cx = self.position.0 as i32 as u32 as u64;
        let cz = self.position.2 as i32 as u32 as u64;
        let mut rng: u64 = ((cx << 32 | cz) ^ salt).wrapping_mul(0x9E3779B97F4A7C15);
        let mut next = || {
            rng = rng
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (rng >> 32) as u32
        };
        let unit = |r: u32| (r >> 8) as f32 / ((1u32 << 24) as f32);

        let mut out = Vec::new();
        for _ in 0..attempts {
            let rx = unit(next());
            let rz = unit(next());
            let x = x0 + rx * CHUNK_SIZE_FLOAT;
            let z = z0 + rz * CHUNK_SIZE_FLOAT;

            let y = self.ground_height(x, z);

            if y < SCATTER_MIN_Y || y > SCATTER_MAX_Y || y < floor || y >= ceiling {
                let _ = next(); // burn an RNG sample so accepted yaw is stable
                continue;
            }

            let yaw = unit(next()) * std::f32::consts::TAU;
            out.push((x, y, z, yaw));
        }
        out
    }
}

impl Chunk for DualContouringChunk {
    fn new(seed: u64, position: (f32, f32, f32), lod: usize) -> Self {
        Self::generate(&WorldGenerator::new(seed), position, lod)
    }

    fn generate(generator: &Arc<WorldGenerator>, position: (f32, f32, f32), _lod: usize) -> Self {
        let mut chunk = Self {
            position,
            coord: ChunkCoord::from_position(position),
            generator: generator.clone(),
            meshes: Vec::new(),
            ground_indices: 0,
            drawn_bounds: ChunkBounds {
                min: (0, 0, 0),
                max: (0, 0, 0),
            },
            water_mesh: None,
        };
        (chunk.meshes, chunk.ground_indices) = chunk.generate_meshes();
        chunk.drawn_bounds = chunk.measure_drawn_bounds();
        chunk.water_mesh = chunk.generate_water_mesh();
        chunk
    }

    fn vertical_range(generator: &WorldGenerator, column: (i32, i32)) -> std::ops::RangeInclusive<i32> {
        // A chunk meshes the ground up to a coarse step (and its skirt)
        // beyond its floor and ceiling.
        let column = generator.column(column.0, column.1);
        let reach = 2.0 * (1 << (LODS - 1)) as f32;
        let chunk = |h: f32| (h / CHUNK_SIZE_FLOAT).floor() as i32;
        // The water lies over ground below it, so in this range too.
        let low = chunk(column.min_height - reach);
        let high = chunk(column.max_height + reach);
        low..=high
    }

    fn is_empty(&self) -> bool {
        self.meshes.is_empty() && self.water_mesh.is_none()
    }

    fn buffer_data(&mut self) {
        // The collider and the drawn bounds are made before this, so the
        // copies in memory are not needed any more: the GPU has them.
        for mesh in &mut self.meshes {
            mesh.buffer_and_release();
        }
        if let Some(water_mesh) = &mut self.water_mesh {
            water_mesh.buffer_and_release();
        }
    }

    fn get_bounds(&self) -> ChunkBounds {
        ChunkBounds {
            min: (
                (self.position.0 * CHUNK_SIZE as f32) as i32,
                (self.position.1 * CHUNK_SIZE as f32) as i32,
                (self.position.2 * CHUNK_SIZE as f32) as i32,
            ),
            max: (
                ((self.position.0 + 1.0) * CHUNK_SIZE as f32) as i32,
                ((self.position.1 + 1.0) * CHUNK_SIZE as f32) as i32,
                ((self.position.2 + 1.0) * CHUNK_SIZE as f32) as i32,
            ),
        }
    }

    fn get_drawn_bounds(&self) -> ChunkBounds {
        self.drawn_bounds
    }

    fn process_line(&mut self, _: &Line, _: &MouseButton) -> bool {
        false
    }

    fn get_position(&self) -> Point3<f32> {
        Point3::new(
            self.position.0 * CHUNK_SIZE_FLOAT,
            self.position.1 * CHUNK_SIZE_FLOAT,
            self.position.2 * CHUNK_SIZE_FLOAT,
        )
    }

    fn get_shader_source() -> (String, String) {
        (
            include_str!("vertex.glsl").to_string(),
            format!(
                "#version 460 core\n{}\n{CLOUD_UNIFORMS}\n{}\n{}\n{}",
                pbr::LIGHTING_GLSL,
                sky::CLOUD_FIELD_GLSL,
                sky::CLOUD_SHADOW_GLSL,
                include_str!("fragment.glsl")
            ),
        )
    }

    fn get_depth_shader_source() -> Option<(String, String)> {
        Some((
            include_str!("depth_vertex.glsl").to_string(),
            depth_pass::EMPTY_FRAGMENT.to_string(),
        ))
    }

    fn get_textures(shader: &Shader) -> Vec<Texture> {
        materials::load(shader)
    }

    fn get_triangle_count(&self) -> usize {
        self.meshes.first().map_or(0, ChunkMesh::get_triangle_count)
    }

    fn scatter_points(&self, salt: u64, attempts: u32) -> Vec<(f32, f32, f32, f32)> {
        self.scatter(salt, attempts)
    }

    fn draw(&self, shader: &Shader, transform: &Matrix4<f32>, distance: f32) {
        if let Some(mesh) = self.meshes.get(Self::lod_at(distance)).or(self.meshes.last()) {
            mesh.render(
                shader,
                &(transform
                    * Matrix4::from_translation(Vector3::new(
                        self.position.0 * CHUNK_SIZE_FLOAT,
                        self.position.1 * CHUNK_SIZE_FLOAT,
                        self.position.2 * CHUNK_SIZE_FLOAT,
                    ))),
                None,
            );
        }
    }

    fn get_vertices(&self) -> Vec<[f32; 3]> {
        // The finest level: the colliders follow the ground closely. Empty
        // once the chunk is buffered (see `buffer_data`).
        if let Some(mesh) = self.meshes.first() {
            mesh.vertices
                .iter()
                .map(|v| [v.position[0], v.position[1], v.position[2]])
                .collect()
        } else {
            Vec::new()
        }
    }

    fn get_indices(&self) -> Vec<[u32; 3]> {
        // The ground without its skirt.
        if let Some(mesh) = self.meshes.first() {
            if let Some(indices) = &mesh.indices {
                let ground = &indices[..self.ground_indices.min(indices.len())];
                return ground.chunks(3).map(|c| [c[0], c[1], c[2]]).collect();
            }
        }
        Vec::new()
    }

    fn render_transparent(
        &self,
        scene: &Scene,
        view_projection: &Matrix4<f32>,
        parent_transform: &Matrix4<f32>,
    ) {
        // Skip water when rendering into the shadow-map FBO or the terrain
        // debug FBO.  The scene knows which FBOs allow the transparent pass.
        if !scene.is_water_render_allowed() {
            return;
        }

        if let Some(water_mesh) = &self.water_mesh {
            if !water_mesh.is_buffered() {
                return;
            }
            let chunk_offset = (
                self.position.0 * CHUNK_SIZE_FLOAT,
                self.position.1 * CHUNK_SIZE_FLOAT,
                self.position.2 * CHUNK_SIZE_FLOAT,
            );
            let model_matrix = parent_transform
                * Matrix4::from_translation(Vector3::new(
                    chunk_offset.0,
                    chunk_offset.1,
                    chunk_offset.2,
                ));
            let depth_capture = scene.is_water_depth_capture();
            let opaque = hdr::opaque_scene().filter(|_| !depth_capture);

            WATER_SHADER.with(|ws| {
                ws.bind();
                ws.set_uniform_mat4("viewProjection", view_projection);
                let inverse = view_projection.invert().unwrap_or(Matrix4::identity());
                ws.set_uniform_mat4("inverseViewProjection", &inverse);
                ws.set_uniform_1f("uTime", START.elapsed().as_secs_f32());
                match Lighting::from_scene(scene) {
                    Some(lighting) => lighting.apply(ws),
                    None => {
                        let eye = scene
                            .get_active_camera()
                            .map(|c| c.get_camera().get_eye_position())
                            .unwrap_or(Point3::new(0.0, 0.0, 0.0));
                        Lighting::preview(Vector3::new(0.3, 0.8, 0.2), eye).apply(ws);
                    }
                }
                ws.set_uniform_1i("sceneColor", SCENE_COLOR_UNIT as i32);
                ws.set_uniform_1i("sceneDepth", SCENE_DEPTH_UNIT as i32);
                ws.set_uniform_1i("hasScene", opaque.is_some() as i32);
                unsafe {
                    if let Some(opaque) = opaque {
                        gl::ActiveTexture(gl::TEXTURE0 + SCENE_COLOR_UNIT);
                        gl::BindTexture(gl::TEXTURE_2D, opaque.color);
                        gl::ActiveTexture(gl::TEXTURE0 + SCENE_DEPTH_UNIT);
                        gl::BindTexture(gl::TEXTURE_2D, opaque.depth);
                        gl::ActiveTexture(gl::TEXTURE0);
                    }
                    gl::Disable(gl::CULL_FACE);
                    if depth_capture {
                        // Depth-capture mode: write the water surface's actual
                        // depth values into the FBO, ignoring whatever terrain
                        // depth is already there.  GL_ALWAYS + DepthMask=TRUE
                        // means every water fragment stamps its depth, giving a
                        // clean "water depth map" for the F10 debug panel.
                        gl::DepthMask(gl::TRUE);
                        gl::DepthFunc(gl::ALWAYS);
                    } else {
                        // Normal render: the shader composes the water with
                        // the opaque scene under it itself; alpha only fades
                        // it out where it thins to nothing at the shore.
                        gl::Enable(gl::BLEND);
                        gl::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA);
                        gl::DepthMask(gl::TRUE);
                        gl::DepthFunc(gl::LESS);
                    }
                }
                water_mesh.render(ws, &model_matrix, None);
                unsafe {
                    gl::DepthMask(gl::TRUE);
                    gl::DepthFunc(gl::LESS); // restore default
                    if !depth_capture {
                        gl::Disable(gl::BLEND);
                    }
                    if opaque.is_some() {
                        for unit in [SCENE_COLOR_UNIT, SCENE_DEPTH_UNIT] {
                            gl::ActiveTexture(gl::TEXTURE0 + unit);
                            gl::BindTexture(gl::TEXTURE_2D, 0);
                        }
                        gl::ActiveTexture(gl::TEXTURE0);
                    }
                    // CULL_FACE is re-enabled by the terrain opaque pass each frame.
                }
            });
        }
    }
}

impl Component for DualContouringChunk {
    fn update(&mut self, _: &mut Scene, _: &mut Entity, _: f64) {}

    fn render(&self, _: &Scene, _: &Entity, _: &Matrix4<f32>, _: &Matrix4<f32>) {
        // Drawn by the terrain, with the other chunks (see `Chunk::draw`).
    }

    fn handle_event(&mut self, _: &mut Glfw, _: &mut glfw::Window, _: &WindowEvent) {}
}

impl VertexAttributes for Vertex {
    fn get_vertex_attributes() -> Vec<(usize, GLuint)> {
        vec![(3, gl::FLOAT), (3, gl::FLOAT), (3, gl::FLOAT)]
    }
}

#[cfg(test)]
mod tests {
    use super::super::density::DensityField;
    use super::*;

    #[test]
    fn levels_get_coarser_with_distance() {
        assert_eq!(DualContouringChunk::lod_at(0.0), 0);
        assert_eq!(DualContouringChunk::lod_at(200.0), 1);
        assert_eq!(DualContouringChunk::lod_at(400.0), 2);
        assert_eq!(DualContouringChunk::lod_at(5000.0), LODS - 1);
    }

    #[test]
    fn neighbours_meet_at_their_edge_on_every_level() {
        let left = DualContouringChunk::new(2, (0.0, 0.0, 0.0), 0);
        let right = DualContouringChunk::new(2, (1.0, 0.0, 0.0), 0);
        assert_eq!(left.meshes.len(), LODS);
        let n = CHUNK_SIZE_FLOAT;
        // The ground alone, without the skirts.
        let (fl, fr) = (DensityField::new(&left.generator, left.coord, 8), DensityField::new(&right.generator, right.coord, 8));
        let ground = |field: &DensityField, lod: usize| -> Vec<[f32; 3]> {
            field.surface_nets(1 << lod).unwrap().0.iter().map(|v| v.position).collect()
        };
        for lod in 0..LODS {
            let step = (1 << lod) as f32;
            let theirs = ground(&fr, lod);
            // The left chunk's last row of vertices lies in the right
            // one's first cells: the same points, so the meshes join.
            let shared: Vec<_> = ground(&fl, lod)
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

    /// Whether some triangle of `mesh` (moved `offset` along x) lies over
    /// `(x, z)`.
    fn covers(mesh: &(Vec<Vertex>, Vec<u32>), offset: f32, x: f32, z: f32) -> bool {
        mesh.1.chunks(3).any(|t| {
            let p = [t[0], t[1], t[2]].map(|i| {
                let v = mesh.0[i as usize].position;
                (v[0] + offset, v[2])
            });
            let side = |a: (f32, f32), b: (f32, f32)| (b.0 - a.0) * (z - a.1) - (b.1 - a.1) * (x - a.0);
            let s = [side(p[0], p[1]), side(p[1], p[2]), side(p[2], p[0])];
            s.iter().all(|&d| d >= 0.0) || s.iter().all(|&d| d <= 0.0)
        })
    }

    #[test]
    fn neighbours_on_any_levels_leave_no_gap_between_them() {
        let left = DualContouringChunk::new(2, (0.0, 0.0, 0.0), 0);
        let right = DualContouringChunk::new(2, (1.0, 0.0, 0.0), 0);
        let (fl, fr) = (DensityField::new(&left.generator, left.coord, 8), DensityField::new(&right.generator, right.coord, 8));
        let left: Vec<_> = (0..LODS).map(|lod| fl.surface_nets(1 << lod).unwrap()).collect();
        let right: Vec<_> = (0..LODS).map(|lod| fr.surface_nets(1 << lod).unwrap()).collect();
        for (a, mesh_a) in left.iter().enumerate() {
            for (b, mesh_b) in right.iter().enumerate() {
                for k in 0..60 {
                    let z = 20.0 + k as f32 * 0.5;
                    for x in [127.0, 127.5, 128.0, 128.5, 129.0] {
                        assert!(
                            covers(mesh_a, 0.0, x, z) || covers(mesh_b, CHUNK_SIZE_FLOAT, x, z),
                            "gap at ({x}, {z}) between levels {a} and {b}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_finest_level_follows_the_ground() {
        let chunk = DualContouringChunk::new(2, (0.0, 0.0, 0.0), 0);
        let mesh = &chunk.meshes[0];
        let indices = mesh.indices.as_ref().unwrap();
        // The ground (not the skirt) lies where scatter and water put it.
        for &i in &indices[..chunk.ground_indices] {
            let v = mesh.vertices[i as usize];
            let [x, y, z] = v.position;
            assert!((y - chunk.ground_height(x, z)).abs() < 0.75, "{:?}", v.position);
            let length = v.normal.iter().map(|c| c * c).sum::<f32>().sqrt();
            assert!((length - 1.0).abs() < 1e-3 && v.normal[1] > 0.0);
        }
        // Triangles face out of the ground, as the normals do.
        for t in indices[..chunk.ground_indices].chunks(3) {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| mesh.vertices[i as usize]);
            let e1: [f32; 3] = std::array::from_fn(|k| b.position[k] - a.position[k]);
            let e2: [f32; 3] = std::array::from_fn(|k| c.position[k] - a.position[k]);
            let face_y = e1[2] * e2[0] - e1[0] * e2[2];
            assert!(face_y >= -1e-4, "triangle facing down");
        }
        // The collider is the ground alone.
        assert_eq!(chunk.get_indices().len() * 3, chunk.ground_indices);
        assert!(indices.len() > chunk.ground_indices, "no skirt");
    }

    #[test]
    fn the_drawn_bounds_hold_every_level_and_are_flatter_than_the_chunk() {
        let chunk = DualContouringChunk::new(2, (1.0, 0.0, -1.0), 0);
        let b = chunk.get_drawn_bounds();
        for mesh in &chunk.meshes {
            for v in &mesh.vertices {
                let p = [v.position[0] + 128.0, v.position[1], v.position[2] - 128.0];
                assert!(p[0] >= b.min.0 as f32 && p[0] <= b.max.0 as f32);
                assert!(p[1] >= b.min.1 as f32 && p[1] <= b.max.1 as f32);
                assert!(p[2] >= b.min.2 as f32 && p[2] <= b.max.2 as f32);
            }
        }
        assert!(b.max.1 - b.min.1 < CHUNK_SIZE as i32 / 2, "{b:?}");
    }

    #[test]
    fn a_chunk_without_ground_has_no_meshes() {
        let chunk = DualContouringChunk::new(2, (0.0, 3.0, 0.0), 0);
        assert!(chunk.meshes.is_empty());
    }
}
