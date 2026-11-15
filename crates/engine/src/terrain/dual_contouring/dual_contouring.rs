use std::time::Instant;

use cgmath::{Matrix4, Point3, SquareMatrix, Vector3};
use lazy_static::lazy_static;
use gl::types::GLuint;
use glfw::{Glfw, MouseButton, WindowEvent};
use libnoise::prelude::*;

use crate::{
    core::{
        entity::{component::Component, Entity},
        renderer::{
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
    water::{self, WaterVertex},
    ChunkMesh, DualContouringChunk, Vertex, LODS, LOD_DISTANCES, SKIRT_DEPTH, SKIRT_DEPTH_PER_STEP,
};

impl DualContouringChunk {
    /// The terrain's noise at chunk-local column `(x, z)`, 0..1. The ground
    /// is a height field (see `ground_height`).
    fn column_noise(&self, x: i32, z: i32) -> f32 {
        let offset: f64 = 16777216.0;
        let sample_point = (
            (self.position.0 * CHUNK_SIZE_FLOAT) as f64 + x as f64 + offset,
            (self.position.2 * CHUNK_SIZE_FLOAT) as f64 + z as f64 + offset,
        );
        ((1.0 + self.noise.sample([sample_point.0, sample_point.1])) / 2.0) as f32
    }

    /// The ground's meshes, finest first: `LODS` grids over the height
    /// field, a metre apart and then 2, 4 and 8. Their normals all come
    /// from the full-detail heights, so the light on the ground does not
    /// change when a chunk switches between them. Each has a skirt hanging
    /// down from its edges, deeper the coarser it is, which hides the gaps
    /// where a neighbour with another level meets it.
    fn generate_meshes(&self) -> Vec<ChunkMesh<Vertex>> {
        if self.position.1 != 0.0 {
            return Vec::new();
        }
        // The heights one point beyond the chunk on every side, for the
        // normals at its edges.
        let span = CHUNK_SIZE as i32 + 3;
        let mut heights = Vec::with_capacity((span * span) as usize);
        for z in -1..=CHUNK_SIZE as i32 + 1 {
            for x in -1..=CHUNK_SIZE as i32 + 1 {
                heights.push(self.column_noise(x, z) * CHUNK_SIZE_FLOAT - 1.0);
            }
        }
        let height = |x: i32, z: i32| heights[((z + 1) * span + (x + 1)) as usize];
        let normal = |x: i32, z: i32| {
            let n = [height(x - 1, z) - height(x + 1, z), 2.0, height(x, z - 1) - height(x, z + 1)];
            let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            [n[0] / length, n[1] / length, n[2] / length]
        };
        (0..LODS)
            .map(|lod| {
                let step = 1 << lod;
                let cells = CHUNK_SIZE as i32 / step;
                let side = cells + 1;
                let mut vertices = Vec::with_capacity((side * side + 4 * side) as usize);
                for j in 0..side {
                    for i in 0..side {
                        let (x, z) = (i * step, j * step);
                        vertices.push(Vertex {
                            position: [x as f32, height(x, z), z as f32],
                            normal: normal(x, z),
                            color: [0.0, 0.5, 0.1],
                        });
                    }
                }
                let at = |i: i32, j: i32| (j * side + i) as u32;
                let mut indices = Vec::with_capacity((cells * cells * 6 + 4 * cells * 12) as usize);
                for j in 0..cells {
                    for i in 0..cells {
                        indices.extend_from_slice(&[
                            at(i, j),
                            at(i, j + 1),
                            at(i + 1, j),
                            at(i + 1, j),
                            at(i, j + 1),
                            at(i + 1, j + 1),
                        ]);
                    }
                }
                // The skirt: each edge again, lowered, joined to the edge by
                // quads facing both ways.
                let depth = SKIRT_DEPTH + SKIRT_DEPTH_PER_STEP * step as f32;
                let edges: [Vec<(i32, i32)>; 4] = [
                    (0..side).map(|i| (i, 0)).collect(),
                    (0..side).map(|i| (i, cells)).collect(),
                    (0..side).map(|j| (0, j)).collect(),
                    (0..side).map(|j| (cells, j)).collect(),
                ];
                for edge in edges {
                    let first = vertices.len() as u32;
                    for &(i, j) in &edge {
                        let mut lowered = vertices[at(i, j) as usize];
                        lowered.position[1] -= depth;
                        vertices.push(lowered);
                    }
                    for k in 0..edge.len() - 1 {
                        let (top0, top1) = (at(edge[k].0, edge[k].1), at(edge[k + 1].0, edge[k + 1].1));
                        let (low0, low1) = (first + k as u32, first + k as u32 + 1);
                        indices.extend_from_slice(&[
                            top0, low0, top1, top1, low0, low1, // one side
                            top0, top1, low0, top1, low1, low0, // the other
                        ]);
                    }
                }
                ChunkMesh::new(vertices, Some(indices))
            })
            .collect()
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
        let offset: f64 = 16777216.0;
        let n = self.noise.sample([x as f64 + offset, z as f64 + offset]);
        ((1.0 + n) / 2.0) as f32 * CHUNK_SIZE_FLOAT - 1.0
    }

    /// The water surface over this chunk (see `water`). Only the bottom
    /// row of chunks holds the ground, and with it the water.
    fn generate_water_mesh(&self) -> Option<ChunkMesh<WaterVertex>> {
        if self.position.1 != 0.0 {
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
    /// tuples. Only chunks at `y_chunk == 0` (where terrain actually exists)
    /// contribute spots.
    fn scatter(&self, salt: u64, attempts: u32) -> Vec<(f32, f32, f32, f32)> {
        if self.position.1 != 0.0 {
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

            if y < SCATTER_MIN_Y || y > SCATTER_MAX_Y {
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
    fn new(seed: u64, position: (f32, f32, f32), _lod: usize) -> Self {
        let noise = Source::perlin(seed).scale([0.003; 2]).fbm(6, 1.0, 2.0, 0.5);
        let mut chunk = Self {
            position,
            noise,
            meshes: Vec::new(),
            water_mesh: None,
        };
        chunk.meshes = chunk.generate_meshes();
        chunk.water_mesh = chunk.generate_water_mesh();
        chunk
    }

    fn buffer_data(&mut self) {
        for mesh in &mut self.meshes {
            mesh.buffer_data();
        }
        if let Some(water_mesh) = &mut self.water_mesh {
            water_mesh.buffer_data();
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
        // The finest level: the colliders follow the ground closely.
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
        if let Some(mesh) = self.meshes.first() {
            if let Some(indices) = &mesh.indices {
                return indices.chunks(3).map(|c| [c[0], c[1], c[2]]).collect();
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
        for lod in 0..LODS {
            let side = CHUNK_SIZE / (1 << lod) + 1;
            for j in 0..side {
                let a = left.meshes[lod].vertices[j * side + side - 1];
                let b = right.meshes[lod].vertices[j * side];
                assert_eq!(a.position[1], b.position[1]);
                assert_eq!(a.position[2], b.position[2]);
                assert_eq!(a.normal, b.normal);
            }
        }
        // The finest level follows the ground the scatter and water use.
        let v = left.meshes[0].vertices[5 * (CHUNK_SIZE + 1) + 7];
        assert!((v.position[1] - left.ground_height(7.0, 5.0)).abs() < 1e-3);
    }
}
