use std::{
    cmp::max,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Instant,
};

use cgmath::{EuclideanSpace, Matrix4, Point3};
use glfw::MouseButton;
use lazy_static::lazy_static;
use rapier3d::prelude::*;

lazy_static! {
    static ref START: Instant = Instant::now();
}

use crate::core::{
    entity::{
        component::{Component, Property, PropertyValue},
        Entity,
    },
    mouse_picker::MousePicker,
    physics::rigidbody::RigidBody,
    renderer::{
        light::skylight::SkyLight,
        line::Line,
        pbr::Lighting,
        shader::{DynamicVertexArray, Shader, VertexAttributes},
    },
    scene::{scene_file::ComponentData, Scene},
    view_frustum::ViewFrustum,
};

use super::{
    scatter::{Scatter, ScatterInstances, ScatterLayerData},
    Chunk, ChunkBounds, ChunkMesh, Terrain, CHUNK_RADIUS, CHUNK_SIZE, CHUNK_SIZE_FLOAT,
};

/// How many layer-on-chunk plantings happen per frame. Spreads the collider
/// work out when a scatter layer is added to (or changed on) an already
/// loaded terrain, instead of stalling one frame on every chunk at once.
const SCATTER_PLANTINGS_PER_FRAME: usize = 4;

/// Seconds the seed has to stay untouched before the terrain is rebuilt, so
/// typing a number does not regenerate on every digit.
const RESEED_DELAY: f64 = 0.6;

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

impl<T: Chunk + Component + Send + 'static> Terrain<T> {
    pub fn new(seed: u64) -> Self {
        let shader_source = T::get_shader_source();
        let shader = Shader::new(&shader_source.0, &shader_source.1);
        let textures = T::get_textures(&shader);

        Self {
            seed,
            reseed_in: None,
            scatter: Scatter::new(Vec::new()),
            show_colliders: false,
            chunk_receiver: Self::start_chunkloaders(seed),
            shader,
            textures,
            mouse_picker: MousePicker::new(),
        }
    }

    /// Models to scatter over the terrain. Purely optional: the terrain
    /// works without any, and a model that cannot be loaded is skipped.
    pub fn with_scatter(mut self, layers: Vec<ScatterLayerData>) -> Self {
        self.scatter = Scatter::new(layers);
        self
    }

    pub fn scatter(&self) -> &Scatter {
        &self.scatter
    }

    /// For adding, removing and repointing scatter layers at runtime; the
    /// chunks follow on the next update.
    pub fn scatter_mut(&mut self) -> &mut Scatter {
        &mut self.scatter
    }

    /// Generate the origin chunk and start the worker threads for the rest.
    /// They stop on their own once the returned receiver is dropped.
    fn start_chunkloaders(seed: u64) -> Receiver<T> {
        let (tx, rx) = mpsc::channel();
        let origin = T::new(seed, (0.0, 0.0, 0.0), 0);
        tx.send(origin).unwrap();

        let tx1 = tx.clone();
        let tx2 = tx.clone();
        let tx3 = tx.clone();
        let tx4 = tx.clone();
        let _ = thread::spawn(move || Terrain::chunkloader(seed, CHUNK_RADIUS as i32, 1, 1, tx1));
        let _ = thread::spawn(move || Terrain::chunkloader(seed, CHUNK_RADIUS as i32, -1, 1, tx2));
        let _ = thread::spawn(move || Terrain::chunkloader(seed, CHUNK_RADIUS as i32, 1, -1, tx3));
        let _ = thread::spawn(move || Terrain::chunkloader(seed, CHUNK_RADIUS as i32, -1, -1, tx4));
        rx
    }

    /// Throw away every chunk (with its colliders and scattered models) and
    /// generate the terrain again from the current seed.
    fn regenerate(&mut self, scene: &mut Scene, entity: &mut Entity) {
        let children = entity.get_children_mut();
        let mut index = 0;
        while index < children.len() {
            if children[index].get_own_component::<T>().is_some() {
                let mut chunk_entity = children.remove(index);
                chunk_entity.release_resources(&mut scene.physics_engine);
            } else {
                index += 1;
            }
        }
        self.chunk_receiver = Self::start_chunkloaders(self.seed);
    }

    pub fn process_line(&mut self, line: Option<(Line, MouseButton)>) {
        if let Some((line, _button)) = line {
            for _chunk_bounds in ChunkBounds::get_chunk_bounds_on_line(&line) {
                // for chunk in entity.get_with_own_component_mut::<T>() {
                //     let chunk = chunk.get_component_mut::<T>().unwrap();
                //     if chunk.get_bounds() == chunk_bounds {
                //         chunk.process_line(&line, &button);
                //     }
                // }
            }
        }
    }

    fn chunkloader(seed: u64, radius: i32, x_dir: i32, z_dir: i32, tx: Sender<T>) {
        let mut x: i32 = 1;
        let mut z: i32 = 0;

        loop {
            if x > radius {
                break;
            }
            let position = if z_dir > 0 {
                ((x * x_dir) as f32, 0.0, z as f32)
            } else {
                ((z * z_dir) as f32, 0.0, (x * x_dir) as f32)
            };
            let new_chunk = T::new(seed, position, max(x.abs(), z.abs()) as usize);
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

    /// Bring every loaded chunk in line with the scatter layers: plant what
    /// is missing and clear what no layer asks for any more (layer removed,
    /// model or density changed).
    fn sync_scatter(&self, scene: &mut Scene, entity: &mut Entity) {
        let specs = self.scatter.specs();
        let mut planted = 0;
        for chunk_entity in entity.get_children_mut() {
            if chunk_entity.get_own_component::<T>().is_none() {
                continue;
            }
            let mut present = vec![false; specs.len()];
            let mut index = 0;
            while index < chunk_entity.components().len() {
                let layer = chunk_entity.components()[index]
                    .as_any()
                    .downcast_ref::<ScatterInstances>()
                    .map(|instances| specs.iter().position(|spec| instances.matches(spec)));
                match layer {
                    // Not scatter at all.
                    None => {}
                    Some(Some(layer)) if !present[layer] => present[layer] = true,
                    // Stale: no layer wants these instances (any more).
                    Some(_) => {
                        if let Some(mut stale) = chunk_entity.remove_component_at(index) {
                            stale.on_remove(&mut scene.physics_engine);
                        }
                        continue;
                    }
                }
                index += 1;
            }
            for (spec, present) in specs.iter().zip(present) {
                if present || planted >= SCATTER_PLANTINGS_PER_FRAME {
                    continue;
                }
                let Some(chunk) = chunk_entity.get_own_component::<T>() else {
                    break;
                };
                let points = chunk.scatter_points(spec.salt, spec.attempts);
                let chunk_body = chunk_entity
                    .get_own_component::<RigidBody>()
                    .map(|body| body.rigid_body_handle);
                // Added even when no spot was suitable, so the chunk counts
                // as done for this layer.
                let mut instances = ScatterInstances::new(spec, &points);
                instances.add_colliders(&points, scene, chunk_body);
                chunk_entity.add_component(instances);
                planted += 1;
            }
        }
    }

    pub fn get_triangle_count(&self, entity: &Entity) -> usize {
        let mut count = 0;
        for chunk in entity.get_with_own_component::<T>() {
            let chunk = chunk.get_component::<T>().unwrap();
            count += chunk.get_triangle_count();
        }
        count
    }

    pub fn get_shader(&self) -> &Shader {
        &self.shader
    }

    /// Bind the terrain's textures to units 0, 1, ... Chunks call this
    /// before drawing: what is drawn between them (scattered models) uses
    /// the same units.
    pub fn bind_textures(&self) {
        for (i, texture) in self.textures.iter().enumerate() {
            unsafe {
                gl::ActiveTexture(gl::TEXTURE0 + i as u32);
            }
            texture.bind();
        }
    }

    pub fn get_mouse_picker(&self) -> &MousePicker {
        &self.mouse_picker
    }
}

impl<T: Chunk + Component + Send + 'static> Component for Terrain<T> {
    fn update(&mut self, scene: &mut Scene, entity: &mut Entity, delta_time: f64) {
        if let Some(remaining) = self.reseed_in {
            let remaining = remaining - delta_time;
            if remaining > 0.0 {
                self.reseed_in = Some(remaining);
            } else {
                self.reseed_in = None;
                self.regenerate(scene, entity);
            }
        }
        self.scatter.tick(delta_time);
        if let Ok(mut chunk) = self.chunk_receiver.try_recv() {
            chunk.buffer_data();
            let mut chunk_exists = false;
            for existing_chunk in entity.get_with_own_component::<T>() {
                let existing_chunk = existing_chunk.get_component::<T>().unwrap();
                if existing_chunk.get_position() == chunk.get_position() {
                    chunk_exists = true;
                    break;
                }
            }
            if !chunk_exists {
                let mut chunk_entity = Entity::new(&format!(
                    "chunk-{}@{:?}",
                    entity.child_count(),
                    chunk.get_position()
                ));
                let vertices: Vec<Vector> = chunk
                    .get_vertices()
                    .iter()
                    .map(|v| Vector::new(v[0], v[1], v[2]))
                    .collect();
                let position = chunk.get_position();
                let collider = ColliderBuilder::trimesh_with_flags(
                    vertices,
                    chunk.get_indices(),
                    // FIX_INTERNAL_EDGES computes pseudo-normals on shared triangle edges,
                    // preventing ghost-collision impulses that launch the player into the air.
                    TriMeshFlags::FIX_INTERNAL_EDGES,
                )
                .expect("Failed to create collider")
                    .translation(Vector::new(position.x, position.y, position.z))
                    .build();
                // The chunk entity sits at the origin, so its fixed body does
                // too; colliders keep their world placement while becoming
                // removable together with the chunk.
                let chunk_body =
                    RigidBody::new(RigidBodyType::Fixed, scene, &chunk_entity, None);
                scene
                    .physics_engine
                    .add_collider(collider, Some(chunk_body.rigid_body_handle));

                // Scattered models are not part of the chunk itself:
                // `sync_scatter` plants them afterwards.
                chunk_entity.add_component(chunk);
                chunk_entity.add_component(chunk_body);
                chunk_entity.set_transient(true);
                entity.add_child(chunk_entity);
            }
        }
        self.sync_scatter(scene, entity);
        if let Some(camera_component) = scene.get_active_camera() {
            let camera = camera_component.get_camera();
            let projection = camera_component.get_projection();
            self.mouse_picker.update(camera, projection);
        }
    }

    fn update_in_editor(&mut self, scene: &mut Scene, entity: &mut Entity, delta_time: f64) {
        // Chunk streaming has no gameplay side effects, so the editor keeps
        // it running to show the terrain while the simulation is stopped.
        Component::update(self, scene, entity, delta_time);
    }

    fn to_data(&self) -> Option<ComponentData> {
        Some(ComponentData::Terrain {
            seed: self.seed,
            scatter: self.scatter.to_data(),
        })
    }

    fn properties(&self) -> Vec<Property> {
        let mut properties = vec![
            Property::int("seed", self.seed as i64),
            Property::bool("show colliders", self.show_colliders),
        ];
        properties.extend(self.scatter.properties());
        properties
    }

    fn set_property(&mut self, name: &str, value: PropertyValue) {
        if self.scatter.set_property(name, &value) {
            return;
        }
        match (name, value) {
            ("seed", PropertyValue::Int(v)) => {
                // Rebuilding the terrain is deferred to `update`, which has
                // the scene at hand.
                let seed = v.max(0) as u64;
                if seed != self.seed {
                    self.seed = seed;
                    self.reseed_in = Some(RESEED_DELAY);
                }
            }
            ("show colliders", PropertyValue::Bool(v)) => self.show_colliders = v,
            _ => {}
        }
    }

    fn visible_collider_bodies(&self, entity: &Entity) -> Vec<RigidBodyHandle> {
        if !self.show_colliders {
            return Vec::new();
        }
        // Each chunk child owns a fixed body carrying its mesh and scatter
        // colliders.
        entity
            .get_children()
            .iter()
            .filter_map(|chunk| chunk.get_own_component::<RigidBody>())
            .map(|rb| rb.rigid_body_handle)
            .collect()
    }

    fn render(
        &self,
        scene: &Scene,
        _: &Entity,
        _: &Matrix4<f32>,
        _: &Matrix4<f32>,
    ) {
        if let Some(skylight) = scene.get_component::<SkyLight>() {
            let light_position = skylight.get_position();
            self.shader.bind();
            self.shader.set_uniform_3f(
                "lightPosition",
                light_position.x,
                light_position.y,
                light_position.z,
            );
            self.shader
                .set_uniform_1f("cloudCoverage", skylight.get_cloud_coverage());
            let sun = skylight.get_sun_direction();
            self.shader.set_uniform_3f("sunDirection", sun.x, sun.y, sun.z);
            self.shader.set_uniform_1f(
                "uTime",
                START.elapsed().as_secs_f32(),
            );

            let mut bound_fbo = 0;
            unsafe {
                gl::GetIntegerv(gl::DRAW_FRAMEBUFFER_BINDING, &mut bound_fbo);
            }
            self.shader
                .set_uniform_1i("isShadowPass", if bound_fbo != 0 { 1 } else { 0 });
            if let Some(lighting) = Lighting::from_scene(scene) {
                lighting.apply(&self.shader);
            }
            // The chunks are child entities and draw themselves right after
            // this, with the uniforms set here (see `bind_textures`).
        }
    }

    fn render_transparent(
        &self,
        scene: &Scene,
        entity: &Entity,
        view_projection: &Matrix4<f32>,
        parent_transform: &Matrix4<f32>,
    ) {
        // Transparent pass — water and other overlays. Runs after every
        // entity's opaque geometry is in the depth buffer (terrain, player,
        // props), so water correctly depth-tests against all of them.
        for chunk in entity.get_with_own_component::<T>() {
            if let Some(chunk) = chunk.get_component::<T>() {
                if ViewFrustum::is_bounds_in_frustum_matrix(view_projection, chunk.get_bounds()) {
                    Chunk::render_transparent(chunk, scene, view_projection, parent_transform);
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
