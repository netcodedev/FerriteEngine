use std::{
    collections::HashSet,
    sync::{
        mpsc::{self, Receiver},
        Arc,
    },
    time::{Duration, Instant},
};

use cgmath::{EuclideanSpace, InnerSpace, Matrix4, Point3};
use glfw::MouseButton;
use image::RgbaImage;
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
        hdr,
        light::{
            light::{Light, LightSource},
            skylight::SkyLight,
        },
        line::Line,
        pbr::Lighting,
        shader::{self, DynamicVertexArray, Shader, VertexAttributes},
        weather::exposure,
        depth_pass,
    },
    physics::physics_engine::{collider_tag, PhysicsEngine},
    scene::{scene_file::ComponentData, Scene},
    view_frustum::ViewFrustum,
};

use super::{
    generator::{map::MapLayer, WorldGenerator, WorldSettings},
    streaming::{ColumnKey, StreamRadius, Streamer},
    scatter::{Scatter, ScatterInstances, ScatterLayerData, TakenInstance},
    Chunk, ChunkBounds, ChunkMesh, FarWater, Terrain, CHUNK_SIZE, CHUNK_SIZE_FLOAT,
};

/// The tunnels' lamps: warm white, how bright, how far each reaches, and how
/// near the camera one must be to be lit at all.
const TUNNEL_LAMP_COLOR: [f32; 3] = [1.0, 0.86, 0.66];
const TUNNEL_LAMP_INTENSITY: f32 = 6.0;
const TUNNEL_LAMP_RANGE: f32 = 13.0;
const TUNNEL_LAMP_REACH: f32 = 160.0;

/// Under the ground, how far inside the loaded chunks' reach the horizon
/// is left out: where it is sunk all the way (see the terrain's vertex
/// shader).
const HORIZON_HOLE_MARGIN: f32 = 170.0;

/// Pixels per side of the world map (see `WorldMap`).
const MAP_PIXELS: u32 = 256;

/// A picture of the whole world from above (see `generator::map`), made on
/// a thread of its own whenever the world or the layer shown changes.
pub(super) struct WorldMap {
    layer: MapLayer,
    /// The picture, and a number that changes with every new one.
    image: Option<(u64, Arc<RgbaImage>)>,
    version: u64,
    /// Being made: for which version.
    pending: Option<(u64, Receiver<RgbaImage>)>,
    /// Whether anyone asked for it (the editor does); none is made before.
    wanted: bool,
}

impl WorldMap {
    fn new() -> Self {
        Self {
            layer: MapLayer::Height,
            image: None,
            version: 0,
            pending: None,
            wanted: false,
        }
    }

    /// Start over for a changed world or layer.
    fn invalidate(&mut self) {
        self.version += 1;
        self.pending = None;
    }

    fn update(&mut self, generator: &Arc<WorldGenerator>) {
        if let Some((version, rx)) = &self.pending {
            if let Ok(image) = rx.try_recv() {
                self.image = Some((*version, Arc::new(image)));
                self.pending = None;
            }
        }
        let current = self.image.as_ref().is_some_and(|(v, _)| *v == self.version);
        if self.wanted && !current && self.pending.is_none() {
            let (tx, rx) = mpsc::channel();
            let (generator, layer) = (generator.clone(), self.layer);
            std::thread::spawn(move || {
                let _ = tx.send(generator.map_image(MAP_PIXELS, layer));
            });
            self.pending = Some((self.version, rx));
        }
    }
}

/// Main-thread time per frame for adding the chunks the workers made
/// (uploading their meshes, inserting their colliders); the rest waits for
/// the next frame. At least one column is added per frame.
const UPLOAD_BUDGET: Duration = Duration::from_millis(2);

/// How many layer-on-chunk plantings happen per frame. Spreads the collider
/// work out when a scatter layer is added to (or changed on) an already
/// loaded terrain, instead of stalling one frame on every chunk at once.
const SCATTER_PLANTINGS_PER_FRAME: usize = 4;

/// Seconds the seed has to stay untouched before the terrain is rebuilt, so
/// typing a number does not regenerate on every digit.
const RESEED_DELAY: f64 = 0.6;

/// A chunk as the loader threads send it: generated, with its collider
/// built there too (the trimesh and its edge fixing are the slow part of
/// adding a chunk), so the main thread only has to insert them.
pub(super) struct LoadedChunk<T> {
    chunk: T,
    /// `None` for a chunk without ground (only water).
    collider: Option<Collider>,
}

impl<T: Chunk> LoadedChunk<T> {
    fn new(chunk: T) -> Self {
        let vertices: Vec<Vector> = chunk
            .get_vertices()
            .iter()
            .map(|v| Vector::new(v[0], v[1], v[2]))
            .collect();
        let position = chunk.get_position();
        let indices = chunk.get_indices();
        if vertices.is_empty() || indices.is_empty() {
            return Self { chunk, collider: None };
        }
        let collider = ColliderBuilder::trimesh_with_flags(
            vertices,
            indices,
            // FIX_INTERNAL_EDGES computes pseudo-normals on shared triangle edges,
            // preventing ghost-collision impulses that launch the player into the air.
            TriMeshFlags::FIX_INTERNAL_EDGES,
        );
        let collider = match collider {
            Ok(builder) => Some(
                builder
                    .translation(Vector::new(position.x, position.y, position.z))
                    .user_data(collider_tag::TERRAIN)
                    .build(),
            ),
            Err(e) => {
                log::warn!("terrain chunk at {position:?} has no collider: {e:?}");
                None
            }
        };
        Self { chunk, collider }
    }
}

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

    /// Distance from `point` to the nearest point of the bounds, across
    /// the ground (x and z).
    pub fn distance_xz(&self, point: Point3<f32>) -> f32 {
        let gap = |p: f32, min: i32, max: i32| (min as f32 - p).max(p - max as f32).max(0.0);
        let dx = gap(point.x, self.min.0, self.max.0);
        let dz = gap(point.z, self.min.2, self.max.2);
        (dx * dx + dz * dz).sqrt()
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
        Self::with_world(seed, WorldSettings::default())
    }

    /// A terrain shaped by `world` (see `generator::world`).
    pub fn with_world(seed: u64, world: WorldSettings) -> Self {
        let shader_source = T::get_shader_source();
        let shader = Shader::new(&shader_source.0, &shader_source.1);
        let textures = T::get_textures(&shader);
        let depth_shader = T::get_depth_shader_source()
            .map(|(vertex, fragment)| Shader::new(&vertex, &fragment));

        let generator = WorldGenerator::with_settings(seed, world.clone());
        Self {
            seed,
            world,
            map: WorldMap::new(),
            horizon: super::horizon::Horizon::new(),
            streamer: Self::start_streaming(&generator),
            generator,
            reseed_in: None,
            scatter: Scatter::new(Vec::new()),
            show_colliders: false,
            shader,
            depth_shader,
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

    /// What shapes the ground (the same for every chunk of this terrain).
    pub fn generator(&self) -> &Arc<WorldGenerator> {
        &self.generator
    }

    /// The latest picture of the world from above (see `generator::map`)
    /// and its version, which changes with every new picture. Asking
    /// starts making it (and remaking it whenever the world changes); the
    /// first call usually returns `None`.
    pub fn world_map(&mut self) -> Option<(u64, Arc<RgbaImage>)> {
        self.map.wanted = true;
        self.map.image.clone()
    }

    /// Columns of chunks loaded, and columns queued or being made.
    pub fn streaming_status(&self) -> (usize, usize) {
        (self.streamer.loaded_count(), self.streamer.in_flight())
    }

    pub fn scatter(&self) -> &Scatter {
        &self.scatter
    }

    /// For adding, removing and repointing scatter layers at runtime; the
    /// chunks follow on the next update.
    pub fn scatter_mut(&mut self) -> &mut Scatter {
        &mut self.scatter
    }

    /// Start the worker threads that make the columns of chunks around
    /// the camera (see `streaming`). They stop when the streamer is dropped.
    fn start_streaming(generator: &Arc<WorldGenerator>) -> Streamer<Vec<LoadedChunk<T>>> {
        let generator = generator.clone();
        Streamer::new(Streamer::<()>::default_workers(), StreamRadius::DEFAULT, move |(x, z), detail| {
            let lod = if detail { 0 } else { 1 };
            T::vertical_range(&generator, (x, z), detail)
                .filter_map(|y| {
                    let chunk = T::generate(&generator, (x as f32, y as f32, z as f32), lod);
                    (!chunk.is_empty()).then(|| LoadedChunk::new(chunk))
                })
                .collect()
        })
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
        super::bridge_view::clear(scene, entity);
        self.generator = WorldGenerator::with_settings(self.seed, self.world.clone());
        self.streamer = Self::start_streaming(&self.generator);
        self.map.invalidate();
        self.horizon.invalidate();
        exposure::world_changed();
    }

    /// Take the scattered instance whose collider is `collider` out of the
    /// terrain in `scene` (e.g. a tree that was felled): out of its chunk,
    /// with its collider, and for good: a chunk loaded again leaves it out.
    /// `None` if no scattered instance has that collider.
    /// The layer (salt) and base position of the scattered instance with
    /// `collider`, without taking it: what names it on every peer (see
    /// `take_scattered_instance_at`).
    pub fn scattered_instance_of(
        scene: &Scene,
        collider: ColliderHandle,
    ) -> Option<(u64, Point3<f32>)> {
        fn find<T: Chunk + Component + Send + 'static>(
            entity: &Entity,
            collider: ColliderHandle,
        ) -> Option<(u64, Point3<f32>)> {
            if entity.get_own_component::<Terrain<T>>().is_some() {
                let found = entity.get_children().iter().find_map(|chunk| {
                    chunk.components().iter().find_map(|component| {
                        component
                            .as_any()
                            .downcast_ref::<ScatterInstances>()?
                            .instance_of(collider)
                    })
                });
                if found.is_some() {
                    return found;
                }
            }
            entity
                .get_children()
                .iter()
                .find_map(|child| find::<T>(child, collider))
        }
        scene
            .get_entities()
            .iter()
            .find_map(|entity| find::<T>(entity, collider))
    }

    /// Like `take_scattered_instance`, for the instance of the layer with
    /// `salt` that stands at `position` (as `TakenInstance` gives them), so
    /// every peer of a networked game finds the same one. Not loaded yet,
    /// it is still left out for good when its chunk loads; `None` then.
    pub fn take_scattered_instance_at(
        scene: &mut Scene,
        salt: u64,
        position: Point3<f32>,
    ) -> Option<TakenInstance> {
        fn find<T: Chunk + Component + Send + 'static>(
            entity: &Entity,
            salt: u64,
            position: Point3<f32>,
        ) -> Option<ColliderHandle> {
            if entity.get_own_component::<Terrain<T>>().is_some() {
                let found = entity.get_children().iter().find_map(|chunk| {
                    chunk.components().iter().find_map(|component| {
                        component
                            .as_any()
                            .downcast_ref::<ScatterInstances>()?
                            .collider_at(salt, position)
                    })
                });
                if found.is_some() {
                    return found;
                }
            }
            entity
                .get_children()
                .iter()
                .find_map(|child| find::<T>(child, salt, position))
        }
        let collider = scene
            .get_entities()
            .iter()
            .find_map(|entity| find::<T>(entity, salt, position));
        if let Some(taken) = collider.and_then(|c| Self::take_scattered_instance(scene, c)) {
            return Some(taken);
        }
        fn mark<T: Chunk + Component + Send + 'static>(
            entity: &mut Entity,
            salt: u64,
            position: Point3<f32>,
        ) {
            let terrain = entity
                .components_mut()
                .iter_mut()
                .find_map(|c| c.as_any_mut().downcast_mut::<Terrain<T>>());
            if let Some(terrain) = terrain {
                terrain.scatter.mark_removed(salt, position);
            }
            for child in entity.get_children_mut() {
                mark::<T>(child, salt, position);
            }
        }
        for entity in scene.get_entities_mut() {
            mark::<T>(entity, salt, position);
        }
        None
    }

    pub fn take_scattered_instance(
        scene: &mut Scene,
        collider: ColliderHandle,
    ) -> Option<TakenInstance> {
        fn visit<T: Chunk + Component + Send + 'static>(
            entity: &mut Entity,
            physics: &mut PhysicsEngine,
            collider: ColliderHandle,
        ) -> Option<TakenInstance> {
            if entity.get_own_component::<Terrain<T>>().is_some() {
                let taken = entity.get_children_mut().iter_mut().find_map(|chunk| {
                    chunk.components_mut().iter_mut().find_map(|component| {
                        component
                            .as_any_mut()
                            .downcast_mut::<ScatterInstances>()?
                            .take_instance(collider, physics)
                    })
                });
                if let Some(taken) = taken {
                    if let Some(terrain) = entity.get_component_mut::<Terrain<T>>() {
                        terrain.scatter.mark_removed(taken.spec.salt, taken.position);
                    }
                    return Some(taken);
                }
            }
            entity
                .get_children_mut()
                .iter_mut()
                .find_map(|child| visit::<T>(child, physics, collider))
        }
        let (entities, physics) = scene.entities_and_physics_mut();
        entities
            .iter_mut()
            .find_map(|entity| visit::<T>(entity, physics, collider))
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

    /// Follow the camera: let go of the columns it has left behind and add
    /// the chunks of those the workers have made since the last frame (for
    /// up to `UPLOAD_BUDGET`).
    fn stream(&mut self, scene: &mut Scene, entity: &mut Entity) {
        // Without a camera, the terrain loads around the origin.
        let eye = scene
            .get_active_camera()
            .map(|camera| camera.get_camera().get_eye_position())
            .unwrap_or(Point3::new(0.0, 0.0, 0.0));
        let unload = self.streamer.update(eye.x as f64, eye.z as f64);
        if !unload.is_empty() {
            let unload: HashSet<ColumnKey> = unload.into_iter().collect();
            let children = entity.get_children_mut();
            let mut index = 0;
            while index < children.len() {
                let column = children[index]
                    .get_own_component::<T>()
                    .map(|chunk| chunk.get_bounds())
                    .map(|b| (b.min.0.div_euclid(CHUNK_SIZE as i32), b.min.2.div_euclid(CHUNK_SIZE as i32)));
                if column.is_some_and(|column| unload.contains(&column)) {
                    let mut chunk_entity = children.remove(index);
                    chunk_entity.release_resources(&mut scene.physics_engine);
                } else {
                    index += 1;
                }
            }
            exposure::world_changed();
        }
        for (column, chunks) in self.streamer.receive(UPLOAD_BUDGET) {
            // Made again in detail: it replaces the chunks there.
            self.remove_column(scene, entity, column);
            for LoadedChunk { mut chunk, collider } in chunks {
                chunk.buffer_data();
                let mut chunk_entity = Entity::new(&format!("chunk@{:?}", chunk.get_position()));
                // The chunk entity sits at the origin, so its fixed body does
                // too; colliders keep their world placement while becoming
                // removable together with the chunk.
                let chunk_body = RigidBody::new(RigidBodyType::Fixed, scene, &chunk_entity, None);
                if let Some(collider) = collider {
                    scene
                        .physics_engine
                        .add_collider(collider, Some(chunk_body.rigid_body_handle));
                }
                // Scattered models are not part of the chunk itself:
                // `sync_scatter` plants them afterwards.
                chunk_entity.add_component(chunk);
                chunk_entity.add_component(chunk_body);
                chunk_entity.set_transient(true);
                entity.add_child(chunk_entity);
                exposure::world_changed();
            }
        }
    }

    /// Take the chunks of `column` out (with their colliders and models).
    fn remove_column(&self, scene: &mut Scene, entity: &mut Entity, column: ColumnKey) {
        let children = entity.get_children_mut();
        let mut index = 0;
        while index < children.len() {
            let here = children[index]
                .get_own_component::<T>()
                .map(|chunk| chunk.get_bounds())
                .map(|b| (b.min.0.div_euclid(CHUNK_SIZE as i32), b.min.2.div_euclid(CHUNK_SIZE as i32)));
            if here == Some(column) {
                let mut chunk_entity = children.remove(index);
                chunk_entity.release_resources(&mut scene.physics_engine);
            } else {
                index += 1;
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
                let mut points = chunk.scatter_points(spec.salt, spec.attempts);
                points.retain(|&(x, y, z, _)| {
                    !self.scatter.is_removed(spec.salt, Point3::new(x, y, z))
                });
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

    /// Set the uniforms of the terrain's shader that hold for every chunk:
    /// the sun, sky and weather.
    fn set_uniforms(&self, scene: &Scene) {
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
                .set_uniform_1f("cloudCoverage", skylight.effective_cloud_coverage());
            self.shader
                .set_uniform_1f("cloudType", skylight.effective_cloud_type());
            let sun = skylight.get_sun_direction();
            self.shader.set_uniform_3f("sunDirection", sun.x, sun.y, sun.z);
            self.shader.set_uniform_1f(
                "uTime",
                START.elapsed().as_secs_f32(),
            );

            self.shader
                .set_uniform_1i("isShadowPass", !hdr::draws_to_view() as i32);
            if let Some(lighting) = Lighting::from_scene(scene) {
                lighting.apply(&self.shader);
            }
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
        self.stream(scene, entity);
        {
            let eye = scene
                .get_active_camera()
                .map(|camera| camera.get_camera().get_eye_position())
                .unwrap_or(Point3::new(0.0, 0.0, 0.0));
            super::bridge_view::sync(&self.generator, scene, entity, eye);
        }
        if T::has_horizon() {
            let eye = scene
                .get_active_camera()
                .map(|camera| camera.get_camera().get_eye_position())
                .unwrap_or(Point3::new(0.0, 0.0, 0.0));
            self.horizon.update(&self.generator, eye.x as f64, eye.z as f64);
        }
        shader::delete_released_vertex_arrays();
        self.map.update(&self.generator);
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

    /// The tunnels' lamps near the camera (the others light nothing it
    /// sees: rock lies between).
    fn lights(&self, transform: &Matrix4<f32>, eye: Point3<f32>, out: &mut Vec<LightSource>) {
        let Some(roads) = self.generator.roads() else {
            return;
        };
        let lamp = Light::point(TUNNEL_LAMP_COLOR, TUNNEL_LAMP_INTENSITY, TUNNEL_LAMP_RANGE);
        for &[x, y, z] in roads.tunnel_lamps() {
            let at = Point3::new(x, y, z);
            if (at - eye).magnitude2() > TUNNEL_LAMP_REACH * TUNNEL_LAMP_REACH {
                continue;
            }
            out.extend(lamp.source(&(transform * Matrix4::from_translation(at.to_vec()))));
        }
    }

    fn to_data(&self) -> Option<ComponentData> {
        Some(ComponentData::Terrain {
            seed: self.seed,
            scatter: self.scatter.to_data(),
            world: self.generator.settings().clone(),
        })
    }

    fn properties(&self) -> Vec<Property> {
        let mut properties = vec![
            Property::int("seed", self.seed as i64),
            Property::ranged("world size (km)", (self.world.size / 1000.0) as f32, 4.0, 64.0),
            Property::ranged("mountain height", self.world.mountain_scale, 0.0, 2.0),
            Property::ranged("hill height", self.world.hill_scale, 0.0, 3.0),
            Property::ranged("caves", self.world.caves, 0.0, 2.0),
            Property::ranged("overhangs", self.world.overhangs, 0.0, 2.0),
            Property::bool("rivers and lakes", self.world.rivers),
            Property::bool("roads and settlements", self.world.roads),
            Property::choice(
                "map",
                self.map.layer.name().into(),
                MapLayer::ALL.iter().map(|l| l.name().to_string()).collect(),
            ),
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
            ("world size (km)", PropertyValue::Float(v)) => {
                let size = (v.clamp(1.0, 256.0) as f64 * 1000.0).round();
                if size != self.world.size {
                    self.world.size = size;
                    self.reseed_in = Some(RESEED_DELAY);
                }
            }
            ("mountain height", PropertyValue::Float(v)) => {
                if v.max(0.0) != self.world.mountain_scale {
                    self.world.mountain_scale = v.max(0.0);
                    self.reseed_in = Some(RESEED_DELAY);
                }
            }
            ("hill height", PropertyValue::Float(v)) => {
                if v.max(0.0) != self.world.hill_scale {
                    self.world.hill_scale = v.max(0.0);
                    self.reseed_in = Some(RESEED_DELAY);
                }
            }
            ("caves", PropertyValue::Float(v)) => {
                if v.clamp(0.0, 2.0) != self.world.caves {
                    self.world.caves = v.clamp(0.0, 2.0);
                    self.reseed_in = Some(RESEED_DELAY);
                }
            }
            ("overhangs", PropertyValue::Float(v)) => {
                if v.clamp(0.0, 2.0) != self.world.overhangs {
                    self.world.overhangs = v.clamp(0.0, 2.0);
                    self.reseed_in = Some(RESEED_DELAY);
                }
            }
            ("rivers and lakes", PropertyValue::Bool(v)) => {
                if v != self.world.rivers {
                    self.world.rivers = v;
                    self.reseed_in = Some(RESEED_DELAY);
                }
            }
            ("roads and settlements", PropertyValue::Bool(v)) => {
                if v != self.world.roads {
                    self.world.roads = v;
                    self.reseed_in = Some(RESEED_DELAY);
                }
            }
            ("map", PropertyValue::Text(v)) => {
                if let Some(layer) = MapLayer::from_name(&v) {
                    if layer != self.map.layer {
                        self.map.layer = layer;
                        self.map.invalidate();
                    }
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
        entity: &Entity,
        view_projection: &Matrix4<f32>,
        transform: &Matrix4<f32>,
    ) {
        // Depth alone: no textures, no light.
        let shader = match &self.depth_shader {
            Some(depth_shader) if depth_pass::is_active() => depth_shader,
            _ => {
                self.set_uniforms(scene);
                // The chunks, with the shader and textures set up once for
                // all of them. Scattered models draw after them (as
                // components of the chunk entities) and may use the same
                // texture units.
                self.bind_textures();
                &self.shader
            }
        };
        shader.bind();
        shader.set_uniform_mat4("viewProjection", view_projection);
        unsafe {
            gl::Enable(gl::CULL_FACE);
        }
        // Every pass picks the same level of detail for a chunk, by its
        // distance from the camera: the shadows fall from the ground that
        // is drawn, and the depth prepass matches the shading pass.
        let eye = Lighting::from_scene(scene).map(|lighting| lighting.camera_position);
        for child in entity.get_children() {
            let Some(chunk) = child.get_own_component::<T>() else {
                continue;
            };
            // Culled by the box around what it draws, which is far flatter
            // than the chunk's cube (it matters most for the sun's shadow
            // maps, which see the boxes from the side); the level comes
            // from the chunk's own square.
            if ViewFrustum::is_bounds_in_frustum_matrix(view_projection, chunk.get_drawn_bounds()) {
                let distance = eye.map_or(0.0, |eye| chunk.get_bounds().distance_xz(eye));
                chunk.draw(shader, &(transform * child.local_transform()), distance);
            }
        }
        // The horizon beyond them; it casts no shadow worth its cost.
        if let (Some(mesh), Some(eye), true) = (self.horizon.mesh(), eye, hdr::draws_to_view()) {
            shader.bind();
            shader.set_uniform_1i("isHorizon", 1);
            let load = StreamRadius::DEFAULT.load as f32;
            shader.set_uniform_3f("horizonSink", eye.x, eye.z, load);
            // Under the ground (a tunnel, a cave) the horizon sunk under the
            // chunks would show from below: there it is left out (from
            // above it stands in for chunks still loading).
            let under = eye.y < self.generator.surface_height(eye.x as f64, eye.z as f64) - 0.5;
            shader.set_uniform_1f("horizonHole", if under { load - HORIZON_HOLE_MARGIN } else { 0.0 });
            mesh.render(shader, &Matrix4::from_scale(1.0), None);
            shader.bind();
            shader.set_uniform_1i("isHorizon", 0);
        }
        unsafe {
            gl::Disable(gl::CULL_FACE);
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
                // The drawn bounds hold the water, which may lie above
                // the chunk's own box (see `Chunk::get_drawn_bounds`).
                if ViewFrustum::is_bounds_in_frustum_matrix(view_projection, chunk.get_drawn_bounds()) {
                    Chunk::render_transparent(chunk, scene, view_projection, parent_transform);
                }
            }
        }
        // The water beyond the chunks, after theirs (it lies under it where
        // both are).
        let eye = scene.get_active_camera().map(|camera| camera.get_camera().get_eye_position());
        if let (Some(water), Some(eye), true) = (self.horizon.water(), eye, T::has_horizon()) {
            let load = StreamRadius::DEFAULT.load as f32;
            let under = eye.y < self.generator.surface_height(eye.x as f64, eye.z as f64) - 0.5;
            let far = FarWater {
                centre: [eye.x, eye.z],
                load,
                hole: if under { load - HORIZON_HOLE_MARGIN } else { 0.0 },
            };
            T::render_far_water(water, scene, view_projection, far);
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
        vertex_array.buffer_data(&self.vertices, &self.indices);
        self.vertex_array = Some(vertex_array);
    }

    /// Buffer the mesh and drop its copy in memory: only the GPU's is
    /// left. Its vertices and indices read empty after this.
    pub fn buffer_and_release(&mut self) {
        self.buffer_data();
        self.vertices = Vec::new();
        self.indices = self.indices.as_ref().map(|_| Vec::new());
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
                if self.indices.is_some() {
                    gl::DrawElements(
                        gl::TRIANGLES,
                        vertex_array.get_element_count() as i32,
                        gl::UNSIGNED_INT,
                        std::ptr::null(),
                    );
                } else {
                    gl::DrawArrays(gl::TRIANGLES, 0, vertex_array.get_element_count() as i32);
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
        if let Some(vertex_array) = &self.vertex_array {
            return vertex_array.get_element_count() / 3;
        }
        if let Some(indices) = &self.indices {
            indices.len() / 3
        } else {
            self.vertices.len() / 3
        }
    }
}
