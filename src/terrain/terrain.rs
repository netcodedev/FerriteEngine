use std::{collections::HashMap, sync::mpsc, thread};

use cgmath::{EuclideanSpace, Matrix4, Point3};
use glfw::MouseButton;

use crate::core::{
    entity::{
        component::{camera_component::CameraComponent, Component},
        Entity,
    },
    mouse_picker::MousePicker,
    renderer::{
        line::Line,
        shader::{DynamicVertexArray, Shader, VertexAttributes},
    },
    scene::Scene,
    view_frustum::ViewFrustum,
};

use super::{Chunk, ChunkBounds, ChunkMesh, Terrain, CHUNK_RADIUS, CHUNK_SIZE, CHUNK_SIZE_FLOAT};

impl ChunkBounds {
    pub fn parse(position: cgmath::Vector3<f32>) -> Self {
        let chunk_pos = (
            (position.x / CHUNK_SIZE_FLOAT).floor() as i32,
            (position.y / CHUNK_SIZE_FLOAT).floor() as i32,
            (position.z / CHUNK_SIZE_FLOAT).floor() as i32,
        );
        let min = (
            chunk_pos.0 * CHUNK_SIZE as i32,
            chunk_pos.1 * CHUNK_SIZE as i32,
            chunk_pos.2 * CHUNK_SIZE as i32,
        );
        let max = (
            (chunk_pos.0 + 1) * CHUNK_SIZE as i32,
            (chunk_pos.1 + 1) * CHUNK_SIZE as i32,
            (chunk_pos.2 + 1) * CHUNK_SIZE as i32,
        );
        ChunkBounds { min, max }
    }

    pub fn contains(&self, position: cgmath::Point3<f32>) -> bool {
        position.x >= self.min.0 as f32
            && position.x < self.max.0 as f32
            && position.y >= self.min.1 as f32
            && position.y < self.max.1 as f32
            && position.z >= self.min.2 as f32
            && position.z < self.max.2 as f32
    }

    pub fn center(&self) -> Point3<f32> {
        Point3::new(
            (self.min.0 + self.max.0) as f32 / 2.0,
            (self.min.1 + self.max.1) as f32 / 2.0,
            (self.min.2 + self.max.2) as f32 / 2.0,
        )
    }

    pub fn get_chunk_bounds_on_line(line: &Line) -> Vec<ChunkBounds> {
        let mut bounds = Vec::new();
        let current_chunk = ChunkBounds::parse(line.position.to_vec());
        let step_size = 0.1;
        for i in 0..(line.length / step_size) as i32 {
            let position = line.position + line.direction * (i as f32 * step_size);
            let chunk = ChunkBounds::parse(position.to_vec());
            if current_chunk.contains(position) {
                continue;
            }
            if !bounds.contains(&chunk) {
                bounds.push(chunk);
            }
        }
        if !bounds.contains(&current_chunk) {
            bounds.push(current_chunk);
        }
        bounds
    }
}

impl<T: Chunk + Send + 'static> Terrain<T> {
    pub fn new(seed: u64) -> Self {
        let (tx, rx) = mpsc::channel();
        let origin = T::new(seed, (0.0, 0.0, 0.0), 0);
        tx.send(origin).unwrap();
        let shader_source = T::get_shader_source();
        let shader = Shader::new(&shader_source.0, &shader_source.1);

        let tx1 = tx.clone();
        let tx2 = tx.clone();
        let tx3 = tx.clone();
        let tx4 = tx.clone();
        let _ = thread::spawn(move || Terrain::chunkloader(seed, CHUNK_RADIUS as i32, 1, 1, tx1));
        let _ = thread::spawn(move || Terrain::chunkloader(seed, CHUNK_RADIUS as i32, -1, 1, tx2));
        let _ = thread::spawn(move || Terrain::chunkloader(seed, CHUNK_RADIUS as i32, 1, -1, tx3));
        let _ = thread::spawn(move || Terrain::chunkloader(seed, CHUNK_RADIUS as i32, -1, -1, tx4));

        Self {
            chunks: HashMap::<ChunkBounds, T>::new(),
            chunk_receiver: rx,
            shader,
            textures: T::get_textures(),
            mouse_picker: MousePicker::new(),
        }
    }

    pub fn process_line(&mut self, line: Option<(Line, MouseButton)>) {
        if let Some((line, button)) = line {
            for chunk_bounds in ChunkBounds::get_chunk_bounds_on_line(&line) {
                if let Some(chunk) = self.chunks.get_mut(&chunk_bounds) {
                    if chunk.process_line(&line, &button) {
                        break;
                    }
                }
            }
        }
    }

    fn chunkloader(seed: u64, radius: i32, x_dir: i32, z_dir: i32, tx: mpsc::Sender<T>) {
        let mut x: i32 = 1;
        let mut z: i32 = 0;

        loop {
            if x > radius {
                break;
            }
            let new_chunk: T;
            if z_dir > 0 {
                new_chunk = T::new(
                    seed,
                    ((x * x_dir) as f32, 0.0, z as f32),
                    std::cmp::max(x.abs(), z.abs()) as usize,
                );
            } else {
                new_chunk = T::new(
                    seed,
                    ((z * z_dir) as f32, 0.0, (x * x_dir) as f32),
                    std::cmp::max(x.abs(), z.abs()) as usize,
                );
            }

            let result = tx.send(new_chunk);
            if result.is_err() {
                break;
            }

            z = -z;
            if z == -x * z_dir {
                x += 1;
                z = 0;
            } else if z >= 0 {
                z += 1;
            }
        }
    }

    pub fn get_triangle_count(&self) -> usize {
        let mut count = 0;
        for (_, chunk) in &self.chunks {
            count += chunk.get_triangle_count();
        }
        count
    }

    pub fn get_mouse_picker(&self) -> &MousePicker {
        &self.mouse_picker
    }
}

impl<T: Chunk + Send + 'static> Component for Terrain<T> {
    fn update(&mut self, scene: &mut Scene, _: &mut Entity, _: f64) {
        if let Ok(mut chunk) = self.chunk_receiver.try_recv() {
            chunk.buffer_data();
            self.chunks.insert(chunk.get_bounds(), chunk);
        }
        if let Some(camera_component) = scene.get_component::<CameraComponent>() {
            let camera = camera_component.get_camera();
            let projection = camera_component.get_projection();
            self.mouse_picker.update(camera, projection);
        }
    }

    fn render(&self, scene: &Scene, parent_transform: &Matrix4<f32>) {
        if let Some(camera_component) = scene.get_component::<CameraComponent>() {
            let camera = camera_component.get_camera();
            let projection = camera_component.get_projection();
            let view_projection = projection.get_matrix() * camera.get_matrix();
            for (i, texture) in self.textures.iter().enumerate() {
                unsafe {
                    gl::ActiveTexture(gl::TEXTURE0 + i as u32);
                }
                texture.bind();
            }
            for (_, chunk) in &self.chunks {
                if ViewFrustum::is_bounds_in_frustum(projection, camera, chunk.get_bounds()) {
                    chunk.render(parent_transform, &view_projection, &self.shader);
                }
            }
            for (i, _) in self.textures.iter().enumerate() {
                unsafe {
                    gl::ActiveTexture(gl::TEXTURE0 + i as u32);
                    gl::BindTexture(gl::TEXTURE_2D, 0);
                }
            }
        }
    }

    fn handle_event(
        &mut self,
        glfw: &mut glfw::Glfw,
        window: &mut glfw::Window,
        event: &glfw::WindowEvent,
    ) {
        let line = self.mouse_picker.handle_event(glfw, window, event);
        self.process_line(line);
    }
}

impl<T: VertexAttributes + Clone> ChunkMesh<T> {
    pub fn new(vertices: Vec<T>, indices: Option<Vec<u32>>) -> Self {
        Self {
            vertex_array: None,
            indices,
            vertices,
        }
    }

    pub fn buffer_data(&mut self) {
        let mut vertex_array = DynamicVertexArray::new();
        vertex_array.buffer_data(&self.vertices, &self.indices.clone());
        self.vertex_array = Some(vertex_array);
    }

    pub fn render(&self, shader: &Shader, transform: &Matrix4<f32>, scale: Option<f32>) {
        unsafe {
            gl::Enable(gl::DEPTH_TEST);
        }
        shader.bind();
        let mut model = transform.clone();
        if let Some(scale) = scale {
            model = model * cgmath::Matrix4::from_scale(scale);
        }
        shader.set_uniform_mat4("model", &model);

        if let Some(vertex_array) = &self.vertex_array {
            vertex_array.bind();
            unsafe {
                if let Some(_) = &self.indices {
                    gl::DrawElements(
                        gl::TRIANGLES,
                        vertex_array.get_element_count() as i32,
                        gl::UNSIGNED_INT,
                        std::ptr::null(),
                    );
                } else {
                    gl::DrawArrays(gl::TRIANGLES, 0, self.vertices.len() as i32);
                }
            }
        }
        unsafe {
            gl::Disable(gl::DEPTH_TEST);
        }
    }

    pub fn is_buffered(&self) -> bool {
        self.vertex_array.is_some()
    }

    pub fn get_triangle_count(&self) -> usize {
        if let Some(indices) = &self.indices {
            indices.len() / 3
        } else {
            self.vertices.len() / 3
        }
    }
}
