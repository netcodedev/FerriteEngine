use std::{
    collections::HashSet,
    sync::{
        mpsc::{self, Receiver},
        Arc,
    },
    time::{Duration, Instant},
};

use cgmath::{EuclideanSpace, Matrix4, Point3};
use glfw::MouseButton;
use image::RgbaImage;
use lazy_static::lazy_static;
use rapier3d::prelude::*;
use ron::Value;

lazy_static! {
    static ref START: Instant = Instant::now();
}

use crate::core::{
    headless,
    entity::{
        component::{Component, Property, PropertyValue},
        Entity,
    },
    gfx,
    mouse_picker::MousePicker,
    physics::rigidbody::RigidBody,
    renderer::{
        depth_pass, hdr, ssao,
        light::{light::LightSource, skylight::SkyLight},
        line::Line,
        pbr::Lighting,
        shader::{self, DynamicVertexArray, Shader, VertexAttributes},
        texture::Texture,
        weather::{exposure, snow_layer},
    },
    physics::physics_engine::{collider_tag, PhysicsEngine},
    scene::{scene_file::ComponentData, Scene},
    view_frustum::{Culled, ViewFrustum},
};

use super::{
    chunk::TerrainChunk,
    columns::ColumnMap,
    far_scatter::FarScatter,
    generators::{self, GeneratorEntry},
    horizon::Horizon,
    scatter::{Scatter, ScatterInstances, ScatterLayerData, TakenInstance},
    source::{ChunkCoord, TerrainSource},
    streaming::{ColumnKey, StreamRadius, Streamer},
    water, ChunkBounds, ChunkMesh, FarWater, CHUNK_SIZE, CHUNK_SIZE_FLOAT,
};

/// Under the ground, how far inside the loaded chunks' reach the horizon
/// is left out: where it is sunk all the way (see the terrain's vertex
/// shader).
const HORIZON_HOLE_MARGIN: f32 = 170.0;

/// Pixels per side of the world map (see `WorldMap`).
const MAP_PIXELS: u32 = 256;

/// Where `waterLine` lies without a sea: far below anything.
const NO_WATER_LINE: f32 = -1.0e6;

/// A picture of the whole world from above (see
/// `TerrainSource::map_image`), made on a thread of its own whenever the
/// world or the layer shown changes.
struct WorldMap {
    /// The layer shown; empty: the source's first.
    layer: String,
    /// The picture, and a number that changes with every new one.
    image: Option<(u64, Arc<RgbaImage>)>,
    version: u64,
    /// Being made: for which version.
    pending: Option<(u64, Receiver<Option<RgbaImage>>)>,
    /// Whether anyone asked for it since the last update (the editor's map
    /// view does, every frame it is shown); none is made otherwise.
    wanted: bool,
}

impl WorldMap {
    fn new() -> Self {
        Self {
            layer: String::new(),
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

    fn update(&mut self, source: &Arc<dyn TerrainSource>) {
        if let Some((version, rx)) = &self.pending {
            if let Ok(image) = rx.try_recv() {
                self.image = image.map(|image| (*version, Arc::new(image)));
                self.pending = None;
            }
        }
        let current = self.image.as_ref().is_some_and(|(v, _)| *v == self.version);
        if self.wanted && !current && self.pending.is_none() {
            let (tx, rx) = mpsc::channel();
            let layers = source.map_layers();
            let layer = if layers.contains(&self.layer) {
                self.layer.clone()
            } else {
                layers.first().cloned().unwrap_or_default()
            };
            let source = source.clone();
            std::thread::spawn(move || {
                let _ = tx.send(source.map_image(MAP_PIXELS, &layer));
            });
            self.pending = Some((self.version, rx));
        }
        // Asked again by whoever still shows it: once nobody does, a changed
        // world no longer makes a new map in the background.
        self.wanted = false;
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
struct LoadedChunk {
    chunk: TerrainChunk,
    /// `None` for a chunk without ground (only water).
    collider: Option<Collider>,
}

impl LoadedChunk {
    fn new(chunk: TerrainChunk) -> Self {
        let (vertices, indices) = chunk.collision_mesh();
        let position = chunk.position();
        if vertices.is_empty() || indices.is_empty() {
            return Self { chunk, collider: None };
        }
        let vertices: Vec<Vector> = vertices.iter().map(|v| Vector::new(v[0], v[1], v[2])).collect();
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

/// What the terrain draws its ground with: its source's programs and
/// textures.
struct Look {
    shader: Shader,
    depth_shader: Option<Shader>,
    /// The horizon's depth: the depth vertex program (the source's or the
    /// standard one) with `shaders::horizon_depth_fragment`.
    horizon_depth_shader: Shader,
    textures: Vec<Texture>,
}

impl Look {
    fn of(source: &dyn TerrainSource) -> Self {
        let shaders = source.shaders();
        let shader = Shader::new(&shaders.vertex, &shaders.fragment);
        let textures = source.load_textures(&shader);
        let horizon_depth_shader = Shader::new(
            shaders.depth_vertex.as_deref().unwrap_or(super::shaders::STANDARD_DEPTH_VERTEX),
            &super::shaders::horizon_depth_fragment(),
        );
        let depth_shader = shaders
            .depth_vertex
            .map(|vertex| Shader::new(&vertex, depth_pass::EMPTY_FRAGMENT));
        Self {
            shader,
            depth_shader,
            horizon_depth_shader,
            textures,
        }
    }
}

/// The terrain of a scene: the world `generator` names (see `generators`)
/// made from `seed` and `settings`, streamed around the camera as chunk
/// entities under the terrain's own.
pub struct Terrain {
    seed: u64,
    /// The world's name (see `generators`).
    generator: String,
    /// Its settings as saved; `()` for its defaults.
    settings: Value,
    /// The world, once it could be made.
    source: Option<Arc<dyn TerrainSource>>,
    /// Why it could not be made.
    error: Option<String>,
    /// `generators::version()` the world was last made at.
    generators_version: u64,
    /// Countdown until the world is made again for edited settings; set to
    /// make it on the next update.
    reseed_in: Option<f64>,
    /// Models scattered over the chunks. Optional; see `scatter`.
    scatter: Scatter,
    /// Editor toggle: draw chunk and scatter colliders as wireframes.
    show_colliders: bool,
    /// The picture of the world the editor shows (see `WorldMap`).
    map: WorldMap,
    /// The ground beyond the chunks (see `horizon`).
    horizon: Horizon,
    /// The scattered models beyond the chunks (see `far_scatter`).
    far_scatter: FarScatter,
    /// Which columns' chunks are loaded (see `columns`): the horizon and
    /// the far water leave those to them.
    loaded: ColumnMap,
    /// Whether the source was still preparing (`TerrainSource::status`)
    /// on the last update: once it is done, the horizon is made again with
    /// what it prepared (roads, say).
    preparing: bool,
    /// Loads the chunks around the camera (see `streaming`).
    streamer: Option<Streamer<Vec<LoadedChunk>>>,
    look: Option<Look>,
    mouse_picker: MousePicker,
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

impl Terrain {
    /// The world `generator` (see `generators`) of `seed` with `settings`
    /// (`()`: its defaults). It is made on the first update: the scripts
    /// that offer it may not be loaded yet.
    pub fn new(seed: u64, generator: impl Into<String>, settings: Value) -> Self {
        Self {
            seed,
            generator: generator.into(),
            settings,
            source: None,
            error: None,
            generators_version: generators::version(),
            reseed_in: Some(0.0),
            scatter: Scatter::new(Vec::new()),
            show_colliders: false,
            map: WorldMap::new(),
            horizon: Horizon::new(),
            far_scatter: FarScatter::new(),
            loaded: ColumnMap::new(),
            preparing: false,
            streamer: None,
            look: None,
            mouse_picker: MousePicker::new(),
        }
    }

    /// The engine's own world (see `hills`) of `seed`.
    pub fn default_world(seed: u64) -> Self {
        Self::new(seed, generators::DEFAULT, Value::Unit)
    }

    /// Models to scatter over the terrain. Purely optional: the terrain
    /// works without any, and a model that cannot be loaded is skipped.
    pub fn with_scatter(mut self, layers: Vec<ScatterLayerData>) -> Self {
        self.scatter = Scatter::new(layers);
        self
    }

    /// The world, once it is made.
    pub fn source(&self) -> Option<&Arc<dyn TerrainSource>> {
        self.source.as_ref()
    }

    /// The world as its own type `S`, if it is one.
    pub fn source_as<S: TerrainSource>(&self) -> Option<&S> {
        self.source.as_ref()?.as_any().downcast_ref::<S>()
    }

    /// The name of the world (see `generators`).
    pub fn generator_name(&self) -> &str {
        &self.generator
    }

    /// The latest picture of the world from above (see
    /// `TerrainSource::map_image`) and its version, which changes with
    /// every new picture. Asking starts making it (and remaking it whenever
    /// the world changes, while it is asked for every frame); the first call
    /// usually returns `None`.
    pub fn world_map(&mut self) -> Option<(u64, Arc<RgbaImage>)> {
        self.map.wanted = true;
        self.map.image.clone()
    }

    /// Columns of chunks loaded, and columns queued or being made.
    pub fn streaming_status(&self) -> (usize, usize) {
        self.streamer
            .as_ref()
            .map_or((0, 0), |streamer| (streamer.loaded_count(), streamer.in_flight()))
    }

    /// What the terrain is still busy with, for a loading indicator: the
    /// world's own work, or loading chunks around the camera. `None` once
    /// it has caught up.
    pub fn loading_status(&self) -> Option<String> {
        if let Some(status) = self.source.as_ref().and_then(|source| source.status()) {
            return Some(status);
        }
        let (loaded, in_flight) = self.streaming_status();
        (in_flight > 0).then(|| format!("Loading terrain ({loaded} of {} columns)", loaded + in_flight))
    }

    pub fn scatter(&self) -> &Scatter {
        &self.scatter
    }

    /// For adding, removing and repointing scatter layers at runtime; the
    /// chunks follow on the next update.
    pub fn scatter_mut(&mut self) -> &mut Scatter {
        &mut self.scatter
    }

    /// The registered world this terrain names.
    fn entry(&self) -> Option<GeneratorEntry> {
        generators::find(&self.generator)
    }

    /// The settings as the inspector shows them: the saved ones, or the
    /// world's defaults.
    fn shown_settings(&self) -> Value {
        match (&self.settings, self.entry()) {
            (Value::Unit, Some(entry)) => entry.defaults(),
            (settings, _) => settings.clone(),
        }
    }

    /// Start the worker threads that make the columns of chunks around
    /// the camera (see `streaming`). They stop when the streamer is dropped.
    fn start_streaming(source: &Arc<dyn TerrainSource>) -> Streamer<Vec<LoadedChunk>> {
        let source = source.clone();
        // Without a GPU (a server) only the colliders are made.
        let drawn = !headless::is_headless();
        Streamer::new(Streamer::<()>::default_workers(), StreamRadius::DEFAULT, move |(x, z), detail| {
            source
                .vertical_range((x, z), detail)
                .filter_map(|y| {
                    let chunk = TerrainChunk::generate_for(&*source, ChunkCoord::new(x, y, z), detail, drawn);
                    (!chunk.is_empty()).then(|| LoadedChunk::new(chunk))
                })
                .collect()
        })
    }

    /// Whether the world has to be made (again): never made, its settings
    /// changed, or the scripts were loaded again and may bring it (anew).
    fn wants_world(&self, delta_time: f64) -> Option<f64> {
        let reloaded = generators::version() != self.generators_version;
        // The engine's own worlds do not change with the scripts.
        let from_scripts = self.source.is_none() || crate::core::script::terrain_generator(&self.generator).is_some();
        match self.reseed_in {
            Some(remaining) => Some(remaining - delta_time),
            None if reloaded && from_scripts => Some(0.0),
            None => None,
        }
    }

    /// Throw away every chunk (with its colliders and scattered models) and
    /// make the world again from the current generator, seed and settings.
    fn regenerate(&mut self, scene: &mut Scene, entity: &mut Entity) {
        self.streamer = None;
        let children = entity.get_children_mut();
        let mut index = 0;
        while index < children.len() {
            if children[index].get_own_component::<TerrainChunk>().is_some() {
                let mut chunk_entity = children.remove(index);
                chunk_entity.release_resources(&mut scene.physics_engine);
            } else {
                index += 1;
            }
        }
        if let Some(old) = self.source.take() {
            old.clear_scene(scene, entity);
        }
        self.generators_version = generators::version();
        self.map.invalidate();
        self.horizon.invalidate();
        self.far_scatter.invalidate();
        exposure::world_changed();
        let made = match self.entry() {
            Some(entry) => entry.create(self.seed, &self.settings),
            None => Err(format!("no world '{}' is loaded", self.generator)),
        };
        match made {
            Ok(source) => {
                self.look = Some(Look::of(&*source));
                self.streamer = Some(Self::start_streaming(&source));
                self.source = Some(source);
                self.error = None;
            }
            Err(e) => {
                // Tried again when other scripts are loaded.
                if self.error.as_ref() != Some(&e) {
                    log::warn!("terrain: {e}");
                }
                self.look = None;
                self.error = Some(e);
            }
        }
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
        fn find(
            entity: &Entity,
            collider: ColliderHandle,
        ) -> Option<(u64, Point3<f32>)> {
            if entity.get_own_component::<Terrain>().is_some() {
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
                .find_map(|child| find(child, collider))
        }
        scene
            .get_entities()
            .iter()
            .find_map(|entity| find(entity, collider))
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
        fn find(
            entity: &Entity,
            salt: u64,
            position: Point3<f32>,
        ) -> Option<ColliderHandle> {
            if entity.get_own_component::<Terrain>().is_some() {
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
                .find_map(|child| find(child, salt, position))
        }
        let collider = scene
            .get_entities()
            .iter()
            .find_map(|entity| find(entity, salt, position));
        if let Some(taken) = collider.and_then(|c| Self::take_scattered_instance(scene, c)) {
            return Some(taken);
        }
        fn mark(
            entity: &mut Entity,
            salt: u64,
            position: Point3<f32>,
        ) {
            let terrain = entity
                .components_mut()
                .iter_mut()
                .find_map(|c| c.as_any_mut().downcast_mut::<Terrain>());
            if let Some(terrain) = terrain {
                terrain.scatter.mark_removed(salt, position);
            }
            for child in entity.get_children_mut() {
                mark(child, salt, position);
            }
        }
        for entity in scene.get_entities_mut() {
            mark(entity, salt, position);
        }
        None
    }

    pub fn take_scattered_instance(
        scene: &mut Scene,
        collider: ColliderHandle,
    ) -> Option<TakenInstance> {
        fn visit(
            entity: &mut Entity,
            physics: &mut PhysicsEngine,
            collider: ColliderHandle,
        ) -> Option<TakenInstance> {
            if entity.get_own_component::<Terrain>().is_some() {
                let taken = entity.get_children_mut().iter_mut().find_map(|chunk| {
                    chunk.components_mut().iter_mut().find_map(|component| {
                        component
                            .as_any_mut()
                            .downcast_mut::<ScatterInstances>()?
                            .take_instance(collider, physics)
                    })
                });
                if let Some(taken) = taken {
                    if let Some(terrain) = entity.get_component_mut::<Terrain>() {
                        terrain.scatter.mark_removed(taken.spec.salt, taken.position);
                    }
                    return Some(taken);
                }
            }
            entity
                .get_children_mut()
                .iter_mut()
                .find_map(|child| visit(child, physics, collider))
        }
        let (entities, physics) = scene.entities_and_physics_mut();
        entities
            .iter_mut()
            .find_map(|entity| visit(entity, physics, collider))
    }

    pub fn process_line(&mut self, line: Option<(Line, MouseButton)>) {
        if let Some((line, _button)) = line {
            // Nothing on the terrain reacts to clicks yet.
            let _ = ChunkBounds::get_chunk_bounds_on_line(&line);
        }
    }

    /// Follow the camera: let go of the columns it has left behind and add
    /// the chunks of those the workers have made since the last frame (for
    /// up to `UPLOAD_BUDGET`).
    fn stream(&mut self, scene: &mut Scene, entity: &mut Entity, eye: Point3<f32>) {
        let t0 = std::time::Instant::now();
        let _probe = StreamProbe(t0);
        let Some(streamer) = &mut self.streamer else {
            return;
        };
        let unload = streamer.update(eye.x as f64, eye.z as f64);
        if !unload.is_empty() {
            let unload: HashSet<ColumnKey> = unload.into_iter().collect();
            let children = entity.get_children_mut();
            let mut index = 0;
            while index < children.len() {
                let column = children[index].get_own_component::<TerrainChunk>().map(TerrainChunk::column);
                if column.is_some_and(|column| unload.contains(&column)) {
                    let mut chunk_entity = children.remove(index);
                    chunk_entity.release_resources(&mut scene.physics_engine);
                } else {
                    index += 1;
                }
            }
            exposure::world_changed();
        }
        // Timed around the uploads, which are what takes the time.
        streamer.receive(UPLOAD_BUDGET, |column, chunks| {
            // Made again in detail: it replaces the chunks there.
            Self::remove_column(scene, entity, column);
            for LoadedChunk { mut chunk, collider } in chunks {
                chunk.buffer_data();
                let mut chunk_entity = Entity::new(&format!("chunk@{:?}", chunk.position()));
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
        });
    }

    /// Take the chunks of `column` out (with their colliders and models).
    fn remove_column(scene: &mut Scene, entity: &mut Entity, column: ColumnKey) {
        let children = entity.get_children_mut();
        let mut index = 0;
        while index < children.len() {
            let here = children[index].get_own_component::<TerrainChunk>().map(TerrainChunk::column);
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
    ///
    /// Returns the columns whose every chunk carries every layer's models:
    /// the far models leave those to them (see `far_scatter`).
    fn sync_scatter(&self, scene: &mut Scene, entity: &mut Entity) -> HashSet<ColumnKey> {
        let mut planted_columns = HashSet::new();
        let mut unplanted_columns = HashSet::new();
        let Some(source) = &self.source else {
            return planted_columns;
        };
        let specs = self.scatter.specs();
        let mut planted = 0;
        for chunk_entity in entity.get_children_mut() {
            let Some(column) = chunk_entity.get_own_component::<TerrainChunk>().map(TerrainChunk::column) else {
                continue;
            };
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
            if present.iter().all(|&p| p) {
                planted_columns.insert(column);
            } else {
                unplanted_columns.insert(column);
            }
            for (spec, present) in specs.iter().zip(present) {
                if present || planted >= SCATTER_PLANTINGS_PER_FRAME {
                    continue;
                }
                let Some(chunk) = chunk_entity.get_own_component::<TerrainChunk>() else {
                    break;
                };
                let mut points = chunk.scatter_points(&**source, &spec.tag, spec.salt, spec.attempts);
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
        planted_columns.retain(|column| !unplanted_columns.contains(column));
        planted_columns
    }

    pub fn get_triangle_count(&self, entity: &Entity) -> usize {
        entity
            .get_children()
            .iter()
            .filter_map(|chunk| chunk.get_own_component::<TerrainChunk>())
            .map(TerrainChunk::get_triangle_count)
            .sum()
    }

    /// The program the ground is drawn with, once the world is made.
    pub fn get_shader(&self) -> Option<&Shader> {
        self.look.as_ref().map(|look| &look.shader)
    }

    /// Bind the terrain's textures to units 0, 1, ... Chunks call this
    /// before drawing: what is drawn between them (scattered models) uses
    /// the same units.
    pub fn bind_textures(&self) {
        let Some(look) = &self.look else {
            return;
        };
        for (i, texture) in look.textures.iter().enumerate() {
            texture.bind_to(i as u32);
        }
    }

    /// Set the uniforms of the terrain's shader that hold for every chunk:
    /// the sun, sky and weather, and the sea's level.
    fn set_uniforms(&self, scene: &Scene, shader: &Shader) {
        let water_line = self.source.as_ref().and_then(|s| s.sea_level()).unwrap_or(NO_WATER_LINE);
        shader.set_uniform_1f("waterLine", water_line);
        if let Some(skylight) = scene.get_component::<SkyLight>() {
            let light_position = skylight.get_position();
            shader.set_uniform_3f("lightPosition", light_position.x, light_position.y, light_position.z);
            shader.set_uniform_1f("cloudCoverage", skylight.effective_cloud_coverage());
            shader.set_uniform_1f("cloudType", skylight.effective_cloud_type());
            let sun = skylight.get_sun_direction();
            shader.set_uniform_3f("sunDirection", sun.x, sun.y, sun.z);
            shader.set_uniform_1f("uTime", START.elapsed().as_secs_f32());
            shader.set_uniform_1i("isShadowPass", !hdr::draws_to_view() as i32);
            if let Some(lighting) = Lighting::from_scene(scene) {
                lighting.apply(shader);
            }
        }
    }

    pub fn get_mouse_picker(&self) -> &MousePicker {
        &self.mouse_picker
    }

    /// Whether the camera at `eye` is under the ground (a tunnel, a cave).
    fn under_ground(&self, eye: Point3<f32>) -> bool {
        self.source
            .as_ref()
            .is_some_and(|source| eye.y < source.surface_height(eye.x as f64, eye.z as f64) - 0.5)
    }
}

fn camera_eye(scene: &Scene) -> Point3<f32> {
    // Without a camera, the terrain loads around the origin.
    scene
        .get_active_camera()
        .map(|camera| camera.get_camera().get_eye_position())
        .unwrap_or(Point3::new(0.0, 0.0, 0.0))
}

impl Component for Terrain {
    fn update(&mut self, scene: &mut Scene, entity: &mut Entity, delta_time: f64) {
        match self.wants_world(delta_time) {
            Some(remaining) if remaining > 0.0 => self.reseed_in = Some(remaining),
            Some(_) => {
                self.reseed_in = None;
                self.regenerate(scene, entity);
            }
            None => {}
        }
        self.scatter.tick(delta_time);
        let eye = camera_eye(scene);
        self.stream(scene, entity, eye);
        if let Some(source) = self.source.clone() {
            source.sync_scene(scene, entity, eye);
            let preparing = source.status().is_some();
            if self.preparing && !preparing {
                self.horizon.refresh();
            }
            self.preparing = preparing;
            // Nothing is drawn without a GPU (a server): no horizon.
            if !headless::is_headless() {
                self.horizon.update(&source, eye.x as f64, eye.z as f64);
            }
            self.map.update(&source);
            snow_layer::set_water_line(source.sea_level().unwrap_or(NO_WATER_LINE));
        }
        shader::delete_released_vertex_arrays();
        let planted = self.sync_scatter(scene, entity);
        let loaded: HashSet<ColumnKey> = entity
            .get_children()
            .iter()
            .filter_map(|child| child.get_own_component::<TerrainChunk>().map(TerrainChunk::column))
            .collect();
        self.loaded.set(eye, &loaded);
        if let Some(source) = self.source.as_ref().filter(|_| !headless::is_headless()) {
            self.far_scatter.update(source, self.horizon.heights(), &self.scatter);
            self.far_scatter.leave_to_chunks(eye, &planted);
        }
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

    /// The world's lights near the camera (lamps).
    fn lights(&self, transform: &Matrix4<f32>, eye: Point3<f32>, out: &mut Vec<LightSource>) {
        let Some(source) = &self.source else {
            return;
        };
        for (at, light) in source.lights(eye) {
            out.extend(light.source(&(transform * Matrix4::from_translation(at.to_vec()))));
        }
    }

    fn to_data(&self) -> Option<ComponentData> {
        Some(ComponentData::Terrain {
            seed: self.seed,
            generator: self.generator.clone(),
            settings: self.settings.clone(),
            scatter: self.scatter.to_data(),
        })
    }

    fn properties(&self) -> Vec<Property> {
        let mut properties = vec![
            Property::int("seed", self.seed as i64),
            Property::choice("world", self.generator.clone(), generators::names()),
        ];
        if let Some(error) = &self.error {
            properties.push(Property::text("world status", error.clone()).read_only());
        }
        if let Some(entry) = self.entry() {
            properties.extend(entry.properties(&self.settings));
        }
        if let Some(layers) = self.source.as_ref().map(|source| source.map_layers()).filter(|l| !l.is_empty()) {
            let shown = if layers.contains(&self.map.layer) {
                self.map.layer.clone()
            } else {
                layers[0].clone()
            };
            properties.push(Property::choice("map", shown, layers));
        }
        properties.push(Property::bool("show colliders", self.show_colliders));
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
            ("world", PropertyValue::Text(v)) => {
                if v != self.generator {
                    self.generator = v;
                    self.settings = Value::Unit;
                    self.reseed_in = Some(0.0);
                }
            }
            ("map", PropertyValue::Text(v)) => {
                if v != self.map.layer {
                    self.map.layer = v;
                    self.map.invalidate();
                }
            }
            ("show colliders", PropertyValue::Bool(v)) => self.show_colliders = v,
            (name, value) => {
                let Some(entry) = self.entry() else {
                    return;
                };
                let mut settings = self.shown_settings();
                if entry.set_property(&mut settings, name, value) && settings != self.shown_settings() {
                    self.settings = settings;
                    self.reseed_in = Some(RESEED_DELAY);
                }
            }
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
        // The chunks' scattered models draw after this, in the same pass.
        ScatterInstances::new_pass();
        let Some(look) = &self.look else {
            return;
        };
        // Depth alone: no textures, no light.
        let shader = match &look.depth_shader {
            Some(depth_shader) if depth_pass::is_active() => depth_shader,
            _ => {
                self.set_uniforms(scene, &look.shader);
                // The chunks, with the shader and textures set up once for
                // all of them. Scattered models draw after them (as
                // components of the chunk entities) and may use the same
                // texture units.
                self.bind_textures();
                &look.shader
            }
        };
        let ground = gfx::PipelineDesc {
            vertex_layouts: crate::core::gfx::Layouts::of(&[DynamicVertexArray::<super::mesher::TerrainVertex>::layout()]),
            depth: gfx::DepthState::LESS,
            cull: gfx::Cull::Back,
            ..Default::default()
        };
        gfx::set_pipeline(shader.pipeline(&ground));
        shader.set_uniform_mat4("viewProjection", view_projection);
        // Every pass picks the same level of detail for a chunk, by its
        // distance from the camera: the shadows fall from the ground that
        // is drawn, and the depth prepass matches the shading pass.
        let eye = Lighting::from_scene(scene).map(|lighting| lighting.camera_position);
        // A chunk drawn at another level than before, or new, casts another
        // shadow than the far cascades (drawn every few frames) hold: they
        // are drawn again. Asked in the first pass of the frame, the nearest
        // cascade's, so the others are drawn after it this very frame.
        if let Some(eye) = eye {
            let mut changed = false;
            for child in entity.get_children() {
                if let Some(chunk) = child.get_own_component::<TerrainChunk>() {
                    changed |= chunk.note_level(chunk.get_bounds().distance_xz(eye));
                }
            }
            if changed {
                scene.request_shadow_redraw();
            }
        }
        for child in entity.get_children() {
            let Some(chunk) = child.get_own_component::<TerrainChunk>() else {
                continue;
            };
            // Culled by the box around what it draws, which is far flatter
            // than the chunk's cube (it matters most for the sun's shadow
            // maps, which see the boxes from the side); the level comes
            // from the chunk's own square.
            let visible = ViewFrustum::is_bounds_in_frustum_matrix(view_projection, chunk.get_drawn_bounds());
            ViewFrustum::count(Culled::Chunk, visible);
            if visible {
                let distance = eye.map_or(0.0, |eye| chunk.get_bounds().distance_xz(eye));
                chunk.draw(shader, &(transform * child.local_transform()), distance);
            }
        }
        // The horizon beyond them; it casts no shadow worth its cost.
        if let (Some(mesh), Some(eye), true) = (self.horizon.mesh(), eye, hdr::draws_to_view()) {
            let depth_only = !std::ptr::eq(shader, &look.shader);
            let shader = if depth_only { &look.horizon_depth_shader } else { shader };
            gfx::set_pipeline(shader.pipeline(&ground));
            if depth_only {
                shader.set_uniform_mat4("viewProjection", view_projection);
            }
            shader.set_uniform_1i("isHorizon", 1);
            let load = StreamRadius::DEFAULT.load as f32;
            shader.set_uniform_3f("horizonSink", eye.x, eye.z, load);
            // Left out where the chunks are loaded.
            self.loaded.bind_loaded(shader);
            // Under the ground (a tunnel, a cave) the horizon under the
            // chunks would show from below: there it is left out within the
            // chunks' reach altogether (from above it stands in for chunks
            // still loading).
            let under = self.under_ground(eye);
            shader.set_uniform_1f("horizonHole", if under { load - HORIZON_HOLE_MARGIN } else { 0.0 });
            mesh.render(shader, &Matrix4::from_scale(1.0), None);
            shader.set_uniform_1i("isHorizon", 0);
            // The models standing on it.
            if !depth_pass::is_active() && !ssao::in_depth_pass() && !under {
                if let Some(lighting) = Lighting::from_scene(scene) {
                    self.far_scatter.draw(view_projection, &lighting, eye);
                }
            }
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
        // The program and what holds for every chunk are set up once.
        let chunks = entity.get_children().iter().filter_map(|child| {
            let chunk = child.get_own_component::<TerrainChunk>()?;
            // The drawn bounds hold the water, which may lie above the
            // chunk's own box.
            let water = chunk.water(parent_transform)?;
            ViewFrustum::is_bounds_in_frustum_matrix(view_projection, chunk.get_drawn_bounds()).then_some(water)
        });
        water::draw(chunks, scene, view_projection, None);
        // The water beyond the chunks, after theirs (it lies under it where
        // both are).
        let eye = scene.get_active_camera().map(|camera| camera.get_camera().get_eye_position());
        if let (Some(far_water), Some(eye)) = (self.horizon.water(), eye) {
            let load = StreamRadius::DEFAULT.load as f32;
            let far = FarWater {
                centre: [eye.x, eye.z],
                load,
                hole: if self.under_ground(eye) { load - HORIZON_HOLE_MARGIN } else { 0.0 },
                loaded: self.loaded.raw(),
            };
            water::draw([(far_water, Matrix4::from_scale(1.0))], scene, view_projection, Some(far));
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

    /// Draw it with the pipeline set (whose program is `shader`'s).
    pub fn render(&self, shader: &Shader, transform: &Matrix4<f32>, scale: Option<f32>) {
        let mut model = transform.clone();
        if let Some(scale) = scale {
            model = model * cgmath::Matrix4::from_scale(scale);
        }
        shader.set_uniform_mat4("model", &model);
        if let Some(vertex_array) = &self.vertex_array {
            vertex_array.draw();
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

#[cfg(test)]
mod world_map_tests {
    use std::sync::Arc;

    use super::WorldMap;
    use crate::terrain::{
        hills::{Hills, HillsSettings},
        source::{TerrainGenerator, TerrainSource},
    };

    #[test]
    fn a_map_is_only_made_while_it_is_asked_for() {
        let source: Arc<dyn TerrainSource> = Hills::create(2, HillsSettings::default());
        let mut map = WorldMap::new();
        map.update(&source);
        assert!(map.pending.is_none(), "nobody asked");

        map.wanted = true;
        map.update(&source);
        assert!(map.pending.is_some());
        assert!(!map.wanted, "asked again by whoever still shows it");

        // The world changes while nobody shows the map: nothing is made.
        map.invalidate();
        map.update(&source);
        assert!(map.pending.is_none());
        // Shown again: made again.
        map.wanted = true;
        map.update(&source);
        assert!(map.pending.is_some());
    }
}

/// Spike probe (see `core::spike_probe`).
struct StreamProbe(std::time::Instant);
impl Drop for StreamProbe {
    fn drop(&mut self) {
        crate::core::spike_probe::add(&crate::core::spike_probe::STREAM_US, self.0);
    }
}
