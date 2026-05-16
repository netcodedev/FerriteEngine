use core::panic;

use cgmath::{Matrix4, Point3, Vector3};
use gl::types::GLuint;
use glfw::{Glfw, MouseButton, WindowEvent};
use libnoise::prelude::*;

use crate::{
    core::{
        entity::{component::Component, Entity},
        renderer::{light::skylight::SkyLight, line::Line, shader::{Shader, VertexAttributes}, texture::Texture},
        scene::Scene,
    },
    terrain::{Chunk, ChunkBounds, Terrain, CHUNK_SIZE, CHUNK_SIZE_FLOAT, USE_LOD},
};

thread_local! {
    static WATER_SHADER: Shader = Shader::new(
        include_str!("water_vertex.glsl"),
        include_str!("water_fragment.glsl"),
    );
}

use fast_surface_nets::{
    ndshape::{AbstractShape, RuntimeShape},
    {surface_nets, SurfaceNetsBuffer},
};

use super::{ChunkMesh, DualContouringChunk, Vertex, WATER_LEVEL};

impl DualContouringChunk {
    fn get_density_at(&self, (x, y, z): (usize, usize, usize)) -> f32 {
        let offset: f64 = 16777216.0;
        let sample_point = (
            (self.position.0 * CHUNK_SIZE_FLOAT) as f64 + x as f64 + offset,
            (self.position.1 * CHUNK_SIZE_FLOAT) as f64 + y as f64 + offset,
            (self.position.2 * CHUNK_SIZE_FLOAT) as f64 + z as f64 + offset,
        );

        let noise = ((1.0 + self.noise.sample([sample_point.0, sample_point.2])) / 2.0) as f32;
        let _iso = ((1.0
            + self
                .cave
                .sample([sample_point.0, sample_point.1, sample_point.2]))
            / 2.0) as f32;
        let height_iso = 1.0 - ((noise) / ((1.0 + y as f32) / CHUNK_SIZE_FLOAT));
        height_iso
    }

    fn generate_mesh(&self) -> ChunkMesh<Vertex> {
        let mut vertices = Vec::<Vertex>::new();
        let mut indices = Vec::<u32>::new();
        let size = (self.chunk_size + 2) as u32;
        let scale_factor = CHUNK_SIZE / self.chunk_size;
        let shape = RuntimeShape::<u32, 3>::new([size, size, size]);
        let mut sdf = vec![0.0; (size * size * size) as usize];
        for i in 0..sdf.len() {
            let [x, y, z] = shape.delinearize(i as u32);
            sdf[i as usize] = self.get_density_at((
                x as usize * scale_factor,
                y as usize * scale_factor,
                z as usize * scale_factor,
            ));
        }
        let mut buffer = SurfaceNetsBuffer::default();
        surface_nets(&sdf, &shape, [0; 3], [size as u32 - 1; 3], &mut buffer);
        for (i, vertex) in buffer.positions.into_iter().enumerate() {
            let normal = buffer.normals[i];
            vertices.push(Vertex {
                position: [
                    vertex[0] * scale_factor as f32,
                    vertex[1] * scale_factor as f32,
                    vertex[2] * scale_factor as f32,
                ],
                normal,
                color: [0.0, 0.5, 0.1],
            });
        }
        for index in buffer.indices {
            indices.push(index);
        }
        ChunkMesh::new(vertices, Some(indices))
    }

    /// Clip the terrain triangle mesh at WATER_LEVEL and project the underwater
    /// portion onto the water plane.  This gives an exact shoreline — no grid
    /// approximation, no overhang into land.
    ///
    /// For each terrain triangle we check how many vertices sit below the water
    /// plane and handle three cases:
    ///   • all 3 below  → emit the triangle projected to y = WATER_LEVEL
    ///   • 2 below      → clip → quad  → 2 triangles
    ///   • 1 below      → clip → 1 triangle
    ///   • 0 below      → skip
    fn generate_water_mesh(&self) -> Option<ChunkMesh<Vertex>> {
        let terrain = self.mesh.as_ref()?;
        let indices = terrain.indices.as_ref()?;

        // Place the water surface a tiny fraction above WATER_LEVEL so it is
        // never coplanar with the terrain vertices that were clipped to that
        // exact height.  With a strict LESS depth test this guarantees the
        // water fragment is always closer to the camera than the terrain below
        // it, eliminating z-fighting without needing polygon-offset or LEQUAL.
        const WATER_SURFACE_Y: f32 = WATER_LEVEL + 0.05;

        // Project a terrain vertex onto the water plane (keep XZ, fix Y).
        let proj = |v: &Vertex| -> Vertex {
            Vertex {
                position: [v.position[0], WATER_SURFACE_Y, v.position[2]],
                normal: [0.0, 1.0, 0.0],
                color: [0.0; 3],
            }
        };

        // Linearly interpolate the XZ crossing point on an edge that crosses
        // WATER_LEVEL (va is below, vb is above — or vice versa).
        let interp = |va: &Vertex, vb: &Vertex| -> Vertex {
            let dy = vb.position[1] - va.position[1];
            let t = if dy.abs() < 1e-6 {
                0.5
            } else {
                (WATER_LEVEL - va.position[1]) / dy
            };
            Vertex {
                position: [
                    va.position[0] + t * (vb.position[0] - va.position[0]),
                    WATER_SURFACE_Y,
                    va.position[2] + t * (vb.position[2] - va.position[2]),
                ],
                normal: [0.0, 1.0, 0.0],
                color: [0.0; 3],
            }
        };

        let mut wv: Vec<Vertex> = Vec::new();
        let mut wi: Vec<u32> = Vec::new();

        let push_tri = |wv: &mut Vec<Vertex>, wi: &mut Vec<u32>, a: Vertex, b: Vertex, c: Vertex| {
            let base = wv.len() as u32;
            wv.push(a);
            wv.push(b);
            wv.push(c);
            wi.extend_from_slice(&[base, base + 1, base + 2]);
        };

        let push_quad = |wv: &mut Vec<Vertex>, wi: &mut Vec<u32>,
                         a: Vertex, b: Vertex, c: Vertex, d: Vertex| {
            let base = wv.len() as u32;
            wv.push(a);
            wv.push(b);
            wv.push(c);
            wv.push(d);
            wi.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        };

        for tri in indices.chunks(3) {
            let v0 = &terrain.vertices[tri[0] as usize];
            let v1 = &terrain.vertices[tri[1] as usize];
            let v2 = &terrain.vertices[tri[2] as usize];

            let b0 = v0.position[1] < WATER_LEVEL;
            let b1 = v1.position[1] < WATER_LEVEL;
            let b2 = v2.position[1] < WATER_LEVEL;

            match (b0, b1, b2) {
                // ── All above water — skip ─────────────────────────────────
                (false, false, false) => {}

                // ── All below — project the whole triangle up ──────────────
                (true, true, true) => {
                    push_tri(&mut wv, &mut wi, proj(v0), proj(v1), proj(v2));
                }

                // ── One vertex below — clip to one triangle ────────────────
                (true, false, false) => {
                    push_tri(&mut wv, &mut wi, proj(v0), interp(v0, v1), interp(v0, v2));
                }
                (false, true, false) => {
                    push_tri(&mut wv, &mut wi, interp(v1, v0), proj(v1), interp(v1, v2));
                }
                (false, false, true) => {
                    push_tri(&mut wv, &mut wi, interp(v2, v0), interp(v2, v1), proj(v2));
                }

                // ── Two vertices below — clip to a quad ────────────────────
                (true, true, false) => {
                    // v0, v1 below; v2 above
                    push_quad(&mut wv, &mut wi,
                        proj(v0), proj(v1), interp(v1, v2), interp(v0, v2));
                }
                (true, false, true) => {
                    // v0, v2 below; v1 above
                    push_quad(&mut wv, &mut wi,
                        proj(v0), interp(v0, v1), interp(v2, v1), proj(v2));
                }
                (false, true, true) => {
                    // v1, v2 below; v0 above
                    push_quad(&mut wv, &mut wi,
                        interp(v1, v0), proj(v1), proj(v2), interp(v2, v0));
                }
            }
        }

        if wv.is_empty() { None } else { Some(ChunkMesh::new(wv, Some(wi))) }
    }

    fn calculate_chunk_size(lod: usize) -> usize {
        let lod = std::cmp::max(
            8,
            std::cmp::min(
                CHUNK_SIZE,
                CHUNK_SIZE / 2usize.pow(if lod > 0 { (lod - 1) as u32 } else { 0 }),
            ),
        );
        if USE_LOD {
            lod
        } else {
            CHUNK_SIZE
        }
    }
}

impl Chunk for DualContouringChunk {
    fn new(seed: u64, position: (f32, f32, f32), lod: usize) -> Self {
        let noise = Source::perlin(seed).scale([0.003; 2]).fbm(6, 1.0, 2.0, 0.5);
        let cave = Source::perlin(seed).scale([0.1; 3]);
        let mut chunk = Self {
            position,
            cave,
            noise,
            chunk_size: DualContouringChunk::calculate_chunk_size(lod),
            mesh: None,
            water_mesh: None,
        };
        chunk.mesh = Some(chunk.generate_mesh());
        chunk.water_mesh = chunk.generate_water_mesh();
        chunk
    }

    fn buffer_data(&mut self) {
        if let Some(mesh) = &mut self.mesh {
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
            include_str!("fragment.glsl").to_string(),
        )
    }

    fn get_textures() -> Vec<Texture> {
        Vec::new()
    }

    fn get_triangle_count(&self) -> usize {
        if let Some(mesh) = &self.mesh {
            mesh.get_triangle_count()
        } else {
            0
        }
    }

    fn get_vertices(&self) -> Vec<[f32; 3]> {
        if let Some(mesh) = &self.mesh {
            mesh.vertices
                .iter()
                .map(|v| [v.position[0], v.position[1], v.position[2]])
                .collect()
        } else {
            Vec::new()
        }
    }

    fn get_indices(&self) -> Vec<[u32; 3]> {
        if let Some(mesh) = &self.mesh {
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

            WATER_SHADER.with(|ws| {
                ws.bind();
                ws.set_uniform_mat4("viewProjection", view_projection);
                ws.set_uniform_3f(
                    "chunkWorldOffset",
                    chunk_offset.0,
                    chunk_offset.1,
                    chunk_offset.2,
                );
                if let Some(skylight) = scene.get_component::<SkyLight>() {
                    let lp = skylight.get_position();
                    ws.set_uniform_3f("lightPosition", lp.x, lp.y, lp.z);
                }
                unsafe {
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
                        // Normal render: water is semi-transparent, sits on top
                        // of the terrain depth buffer, does not overwrite depth.
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
                    // CULL_FACE is re-enabled by the terrain opaque pass each frame.
                }
            });
        }
    }
}

impl Component for DualContouringChunk {
    fn update(&mut self, _: &mut Scene, _: &mut Entity, _: f64) {}

    fn render(
        &self,
        scene: &Scene,
        _: &Entity,
        view_projection: &Matrix4<f32>,
        parent_transform: &Matrix4<f32>,
    ) {
        if let Some(terrain) = scene.get_component::<Terrain<DualContouringChunk>>() {
            let shader = terrain.get_shader();
            if let Some(mesh) = &self.mesh {
                if !mesh.is_buffered() {
                    panic!("Mesh is not buffered");
                }
                shader.bind();
                shader.set_uniform_mat4("viewProjection", &view_projection);
                unsafe {
                    gl::Enable(gl::CULL_FACE);
                }
                // Pass the chunk's stable world-space offset so the vertex
                // shader can compute camera-independent world coordinates.
                shader.set_uniform_3f(
                    "chunkWorldOffset",
                    self.position.0 * CHUNK_SIZE_FLOAT,
                    self.position.1 * CHUNK_SIZE_FLOAT,
                    self.position.2 * CHUNK_SIZE_FLOAT,
                );
                mesh.render(
                    &shader,
                    &(parent_transform
                        * Matrix4::from_translation(Vector3::new(
                            self.position.0 * CHUNK_SIZE_FLOAT,
                            self.position.1 * CHUNK_SIZE_FLOAT,
                            self.position.2 * CHUNK_SIZE_FLOAT,
                        ))),
                    None,
                );
                unsafe {
                    gl::Disable(gl::CULL_FACE);
                }
            }
        }

    }

    fn handle_event(&mut self, _: &mut Glfw, _: &mut glfw::Window, _: &WindowEvent) {}
}

impl VertexAttributes for Vertex {
    fn get_vertex_attributes() -> Vec<(usize, GLuint)> {
        vec![(3, gl::FLOAT), (3, gl::FLOAT), (3, gl::FLOAT)]
    }
}
