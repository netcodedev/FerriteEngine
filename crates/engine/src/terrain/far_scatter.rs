//! The scattered models (trees, rocks, ...) beyond the loaded chunks, out to
//! the horizon's edge (thinned with distance, see `DENSE_REACH`): on the
//! horizon's ground, as flat pictures of the models
//! that turn toward the camera (see `scatter::impostor`).
//!
//! They stand on the very spots the chunks plant them on (see
//! `chunk::scatter_spots`), as many as the source lets grow there from
//! afar (`TerrainSource::far_scatter_chance`), placed on a thread of their
//! own whenever the horizon moves or the layers change. A column's models
//! are drawn here until its chunks are loaded and carry their own (see
//! `leave_to_chunks`), so the two take turns exactly, wherever the chunks
//! have got to.

use std::{
    collections::HashSet,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
        Arc,
    },
};

use cgmath::{Matrix4, Point3};

use crate::core::{
    gfx,
    renderer::{
        pbr::{self, Lighting},
        shader::Shader,
    },
};

use super::{
    chunk::scatter_spots,
    columns::{ColumnMap, COLUMNS_GLSL},
    horizon::{self, HeightGrid, SINK},
    scatter::{impostor::{self, FRAMES}, pick_variant_index, spot_key, Scatter, ScatterModel, ScatterSpec},
    shaders::HAZE_GLSL,
    source::TerrainSource,
    CHUNK_SIZE,
};



/// What a layer is placed by.
#[derive(Clone, PartialEq, Debug)]
struct Layer {
    tag: String,
    salt: u64,
    attempts: u32,
    variants: usize,
}

impl Layer {
    fn of(spec: &ScatterSpec) -> Self {
        Self {
            tag: spec.tag.clone(),
            salt: spec.salt,
            attempts: spec.attempts,
            variants: spec.model.variant_count(),
        }
    }
}

/// Per layer, per variant: where its models stand and how large they are
/// drawn, `[x, y, z, turn, scale]`.
type Placed = Vec<Vec<Vec<[f32; 5]>>>;

/// Out to here from the horizon's centre every model stands; beyond,
/// fewer: a share `(DENSE_REACH / distance)²`, drawn larger to cover as
/// much ground (up to `MAX_SCALE` times). Further out a model is smaller
/// than a pixel: what counts there is how much of the ground the woods
/// cover, not each tree.
const DENSE_REACH: f64 = 2500.0;
const MAX_SCALE: f32 = 2.0;

/// The share of the models kept at `distance` metres from the centre, and
/// how much larger they are drawn.
fn thinning(distance: f64) -> (f32, f32) {
    if distance <= DENSE_REACH {
        return (1.0, 1.0);
    }
    let kept = (DENSE_REACH / distance).powi(2) as f32;
    (kept, kept.sqrt().recip().min(MAX_SCALE))
}

/// A number 0..1 of the spot `(x, z)` alone, apart from those
/// `scatter_spots` draws, deciding whether it is kept when thinned.
fn keep_draw(x: f32, z: f32, salt: u64) -> f32 {
    let mut h = (x.to_bits() as u64 ^ salt).wrapping_mul(0xD6E8FEB86659FD93);
    h ^= (z.to_bits() as u64).wrapping_mul(0xA0761D6478BD642F);
    h ^= h >> 32;
    h = h.wrapping_mul(0x9E3779B97F4A7C15);
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// The models of `layers` as far as `heights` reaches, on its ground
/// (sunk with it, see `horizon::SINK`), thinned with distance (see
/// `DENSE_REACH`), leaving out the instances taken out (`removed`, see
/// `Scatter::removed_spots`). The rows of columns are shared out among
/// threads. `None` once `cancel` is set (the job was superseded).
fn place(
    source: &dyn TerrainSource,
    heights: &HeightGrid,
    layers: &[Layer],
    removed: &HashSet<(u64, [i32; 2])>,
    cancel: &AtomicBool,
) -> Option<Placed> {
    let centre = heights.centre();
    let size = CHUNK_SIZE as f64;
    let reach = horizon::reach();
    let span = (reach / size).ceil() as i32 + 1;
    let (cx, cz) = ((centre[0] / size).floor() as i32, (centre[1] / size).floor() as i32);
    let rows: Vec<i32> = (cz - span..=cz + span).collect();
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 8);
    let row = |z: i32, out: &mut Placed| {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        for (layer, slots) in layers.iter().zip(out.iter_mut()) {
            for x in cx - span..=cx + span {
                let distance = super::streaming::column_distance((x, z), centre[0], centre[1]);
                if distance > reach {
                    continue;
                }
                for (px, pz, grow, turn) in scatter_spots((x, z), layer.salt, layer.attempts) {
                    let (dx, dz) = (px as f64 - centre[0], pz as f64 - centre[1]);
                    let (kept, scale) = thinning(dx.hypot(dz));
                    if kept < 1.0 && keep_draw(px, pz, layer.salt) >= kept {
                        continue;
                    }
                    if !removed.is_empty() && removed.contains(&(layer.salt, spot_key(px, pz))) {
                        continue;
                    }
                    let Some(height) = heights.height(px as f64, pz as f64) else {
                        continue;
                    };
                    if grow >= source.far_scatter_chance(&layer.tag, px as f64, pz as f64, height) {
                        continue;
                    }
                    let variant = pick_variant_index(px, pz, layer.variants);
                    slots[variant].push([px, height - SINK, pz, turn, scale]);
                }
            }
        }
    };
    let empty = || -> Placed { layers.iter().map(|l| vec![Vec::new(); l.variants.max(1)]).collect() };
    let parts: Vec<Placed> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                let rows = &rows;
                let row = &row;
                scope.spawn(move || {
                    let mut out = empty();
                    // Every `threads`th row: near and far rows alike.
                    for &z in rows.iter().skip(t).step_by(threads) {
                        row(z, &mut out);
                    }
                    out
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().expect("far scatter thread")).collect()
    });
    if cancel.load(Ordering::Relaxed) {
        return None;
    }
    let mut all = empty();
    for part in parts {
        for (layer, slots) in part.into_iter().enumerate() {
            for (variant, mut models) in slots.into_iter().enumerate() {
                all[layer][variant].append(&mut models);
            }
        }
    }
    Some(all)
}

/// What the models drawn (or being placed) were placed for.
#[derive(Clone)]
struct Key {
    heights: Arc<HeightGrid>,
    layers: Vec<Layer>,
    /// `Scatter::removed_count`: placed again when an instance is taken.
    removed: usize,
}

impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.heights, &other.heights)
            && self.layers == other.layers
            && self.removed == other.removed
    }
}

/// One variant's models, ready to draw.
struct Batch {
    model: Rc<ScatterModel>,
    variant: usize,
    /// `[x, y, z, turn, scale]` of each model (`instance_layout`).
    instances: gfx::Buffer,
    count: u32,
}

/// A card's corner (0..1 across and up) at location 0.
fn corner_layout() -> gfx::VertexLayout {
    gfx::VertexLayout::packed(0, gfx::StepMode::Vertex, &[gfx::VertexFormat::F32x2])
}

/// A model: where it stands and how it is turned (location 1), and how
/// large it is drawn (location 2).
fn instance_layout() -> gfx::VertexLayout {
    use gfx::VertexFormat::*;
    gfx::VertexLayout::packed(1, gfx::StepMode::Instance, &[F32x4, F32])
}

/// The far models a terrain draws; see the module.
/// A placement on its thread: what for, and the flag that gives it up.
struct Pending {
    key: Key,
    specs: Vec<ScatterSpec>,
    placed: Receiver<Option<Placed>>,
    cancel: Arc<AtomicBool>,
}

impl Pending {
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

pub struct FarScatter {
    made: Option<Key>,
    /// One placement at a time: a superseded one is given up, and the
    /// latest is started once it has stopped.
    pending: Option<Pending>,
    batches: Vec<Batch>,
    /// The corners of a picture's card, shared by every batch.
    corners: Option<gfx::Buffer>,
    /// Which columns around the camera's carry their chunks' own models.
    planted: ColumnMap,
    shader: Option<Shader>,
}

impl FarScatter {
    pub fn new() -> Self {
        Self {
            made: None,
            pending: None,
            batches: Vec::new(),
            corners: None,
            planted: ColumnMap::new(),
            shader: None,
        }
    }

    /// Forget everything (the world changed).
    pub fn invalidate(&mut self) {
        self.made = None;
        self.cancel_pending();
        self.pending = None;
        self.batches.clear();
    }

    /// Give up the placement running, if any (it stops early).
    fn cancel_pending(&self) {
        if let Some(pending) = &self.pending {
            pending.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// How many models are drawn (at most: those near the camera are left
    /// out when drawing).
    pub fn count(&self) -> usize {
        self.batches.iter().map(|b| b.count as usize).sum()
    }

    /// Place the models of `scatter` on the horizon's ground `heights` (if
    /// that changed, or the layers or what was taken out did), and take up
    /// what was placed.
    pub fn update(&mut self, source: &Arc<dyn TerrainSource>, heights: Option<&Arc<HeightGrid>>, scatter: &Scatter) {
        if let Some(pending) = &self.pending {
            match pending.placed.try_recv() {
                Ok(placed) => {
                    let pending = self.pending.take().expect("pending");
                    // Given up: what came (if anything) is not wanted.
                    if let Some(placed) = placed.filter(|_| !pending.cancelled()) {
                        self.upload(&pending.specs, placed);
                        self.made = Some(pending.key);
                    }
                }
                // Placing panicked (reported by the panic hook): keep
                // drawing what there is, and try again when the ground or
                // the layers change.
                Err(TryRecvError::Disconnected) => {
                    let pending = self.pending.take().expect("pending");
                    if !pending.cancelled() {
                        log::error!("the terrain's far models could not be placed");
                        self.made = Some(pending.key);
                    }
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        let Some(heights) = heights else {
            return;
        };
        let specs = scatter.specs();
        let wanted = Key {
            heights: heights.clone(),
            layers: specs.iter().map(Layer::of).collect(),
            removed: scatter.removed_count(),
        };
        let running = self.pending.as_ref().filter(|pending| !pending.cancelled());
        if running.is_some_and(|pending| pending.key == wanted) {
            return;
        }
        if self.made.as_ref() == Some(&wanted) {
            // What is drawn is what is wanted again.
            self.cancel_pending();
            return;
        }
        if wanted.layers.is_empty() {
            self.batches.clear();
            self.made = Some(wanted);
            self.cancel_pending();
            self.pending = None;
            return;
        }
        if self.pending.is_some() {
            // Superseded: given up, and the latest is placed once it has
            // stopped (on a later update).
            self.cancel_pending();
            return;
        }
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let (source, job, removed, flag) = (source.clone(), wanted.clone(), scatter.removed_spots(), cancel.clone());
        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            let placed = place(&*source, &job.heights, &job.layers, &removed, &flag);
            if let Some(placed) = &placed {
                let count: usize = placed.iter().flatten().map(Vec::len).sum();
                log::debug!("far scatter: {count} models placed in {:.0} ms", started.elapsed().as_secs_f64() * 1000.0);
            }
            let _ = tx.send(placed);
        });
        self.pending = Some(Pending {
            key: wanted,
            specs,
            placed: rx,
            cancel,
        });
    }

    /// The chunks of the columns `planted` (around the camera at `eye`)
    /// carry their own models: leave those columns' models to them.
    pub fn leave_to_chunks(&mut self, eye: Point3<f32>, planted: &HashSet<(i32, i32)>) {
        self.planted.set(eye, planted);
    }

    fn upload(&mut self, specs: &[ScatterSpec], placed: Placed) {
        self.batches.clear();
        if crate::core::headless::is_headless() {
            return;
        }
        self.shader.get_or_insert_with(|| {
            Shader::new(
                &format!("#version 460 core\n{COLUMNS_GLSL}\n{}", include_str!("shaders/far_scatter_vertex.glsl")),
                &format!(
                    "#version 460 core\n{}\n{HAZE_GLSL}\n{}",
                    pbr::LIGHTING_GLSL,
                    include_str!("shaders/far_scatter_fragment.glsl")
                ),
            )
        });
        self.corners.get_or_insert_with(|| {
            let corners: [f32; 8] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0];
            gfx::Buffer::from_slice(gfx::BufferUsage::VERTEX, gfx::BufferMemory::Static, &corners)
        });
        for (spec, variants) in specs.iter().zip(placed) {
            for (variant, models) in variants.into_iter().enumerate() {
                if models.is_empty() {
                    continue;
                }
                self.batches.push(Batch {
                    model: spec.model.clone(),
                    variant,
                    instances: gfx::Buffer::from_slice(gfx::BufferUsage::VERTEX, gfx::BufferMemory::Static, &models),
                    count: models.len() as u32,
                });
            }
        }
    }

    /// Draw the models of the columns whose chunks do not carry their own
    /// yet (see the module), seen from `eye`.
    pub fn draw(&self, view_projection: &Matrix4<f32>, lighting: &Lighting, eye: Point3<f32>) {
        let Some(shader) = self.shader.as_ref().filter(|_| !self.batches.is_empty()) else {
            return;
        };
        // Picture the models first: baking binds textures of its own (on
        // unit 2 too, where `planted` goes) and leaves them bound.
        for batch in &self.batches {
            batch.model.impostors();
        }
        let Some(corners) = &self.corners else {
            return;
        };
        // Cards facing the camera, drawn as the ground is (`LEQUAL`: the
        // depth may be drawn already), from both sides.
        let desc = gfx::PipelineDesc {
            vertex_layouts: vec![corner_layout(), instance_layout()],
            topology: gfx::Topology::TriangleStrip,
            depth: gfx::DepthState::LEQUAL,
            ..Default::default()
        };
        gfx::set_pipeline(shader.pipeline(&desc));
        lighting.apply(shader);
        shader.set_uniform_mat4("viewProjection", view_projection);
        shader.set_uniform_3f("viewer", eye.x, eye.y, eye.z);
        self.planted.bind(shader, 2, "planted", "plantedOrigin");
        shader.set_uniform_1i("frames", FRAMES as i32);
        shader.set_uniform_1i("albedoMap", 0);
        shader.set_uniform_1i("normalMap", 1);
        gfx::set_vertex_buffer(0, corners, 0, corner_layout().stride);
        for batch in &self.batches {
            let Some(impostor) = batch.model.impostors().get(batch.variant) else {
                continue;
            };
            let (Some(albedo), Some(normal)) = (&impostor.albedo, &impostor.normal) else {
                continue;
            };
            shader.set_uniform_3f("card", impostor.half_width, impostor.bottom, impostor.top);
            gfx::bind_texture(0, albedo, &impostor::SAMPLER);
            gfx::bind_texture(1, normal, &impostor::SAMPLER);
            gfx::set_vertex_buffer(1, &batch.instances, 0, instance_layout().stride);
            gfx::draw_instanced(0, 4, 0, batch.count);
        }
        gfx::unbind_texture(2, gfx::TextureKind::D2);
        gfx::unbind_texture(1, gfx::TextureKind::D2Array);
        gfx::unbind_texture(0, gfx::TextureKind::D2Array);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::{
        hills::{Hills, HillsSettings},
        horizon,
        source::TerrainGenerator,
    };

    #[test]
    fn far_models_stand_where_the_chunks_plant_them_and_reach_far() {
        let source = Hills::create(3, HillsSettings::default());
        let built = horizon::build(&*source, [0.0, 0.0]);
        let layer = Layer {
            tag: String::new(),
            salt: 77,
            attempts: 30,
            variants: 3,
        };
        let placed = place(&*source, &built.heights, &[layer.clone()], &HashSet::new(), &AtomicBool::new(false)).unwrap();
        let all: Vec<[f32; 5]> = placed[0].iter().flatten().copied().collect();
        assert!(all.len() > 10_000, "{}", all.len());
        let far = all.iter().map(|m| m[0].hypot(m[2])).fold(0.0f32, f32::max);
        assert!(far > horizon::reach() as f32 * 0.9, "{far}");
        // Thinned far away, and drawn larger there.
        let within = |r0: f32, r1: f32| all.iter().filter(|m| (r0..r1).contains(&m[0].hypot(m[2]))).count() as f32;
        let ring = |r0: f32, r1: f32| std::f32::consts::PI * (r1 * r1 - r0 * r0);
        let (near, distant) = (within(1000.0, 2000.0) / ring(1000.0, 2000.0), within(9000.0, 10000.0) / ring(9000.0, 10000.0));
        assert!(distant < near * 0.2, "{near} {distant}");
        assert!(all.iter().filter(|m| m[0].hypot(m[2]) > 9000.0).all(|m| m[4] > 1.5));
        // Out of the water, on the ground drawn.
        for m in all.iter().take(500) {
            let ground = built.heights.height(m[0] as f64, m[2] as f64).unwrap();
            assert!((m[1] - (ground - SINK)).abs() < 1e-3);
        }
        // The spots of a column are the chunks' spots there.
        let near: Vec<(f32, f32)> = scatter_spots((2, -3), 77, 30).map(|(x, z, _, _)| (x, z)).collect();
        let in_column = all.iter().filter(|m| (m[0] / 128.0).floor() == 2.0 && (m[2] / 128.0).floor() == -3.0);
        for m in in_column {
            assert!(near.contains(&(m[0], m[2])));
        }
    }

    #[test]
    fn a_placement_given_up_places_nothing() {
        let source = Hills::create(3, HillsSettings::default());
        let built = horizon::build(&*source, [0.0, 0.0]);
        let layer = Layer {
            tag: String::new(),
            salt: 77,
            attempts: 30,
            variants: 1,
        };
        assert!(place(&*source, &built.heights, &[layer], &HashSet::new(), &AtomicBool::new(true)).is_none());
    }

    #[test]
    fn instances_taken_out_are_not_drawn_far_away() {
        let source = Hills::create(3, HillsSettings::default());
        let built = horizon::build(&*source, [0.0, 0.0]);
        let layer = Layer {
            tag: String::new(),
            salt: 77,
            attempts: 30,
            variants: 1,
        };
        let spots = |placed: &Placed| -> Vec<(f32, f32)> {
            placed[0][0].iter().map(|m| (m[0], m[2])).collect()
        };
        let before = spots(&place(&*source, &built.heights, &[layer.clone()], &HashSet::new(), &AtomicBool::new(false)).unwrap());
        let taken = before[before.len() / 2];
        // Felled where its chunk planted it: same spot, the chunk's height.
        let mut scatter = Scatter::new(Vec::new());
        scatter.mark_removed(77, Point3::new(taken.0, 12.345, taken.1));
        let after = spots(&place(&*source, &built.heights, &[layer], &scatter.removed_spots(), &AtomicBool::new(false)).unwrap());
        assert_eq!(after.len(), before.len() - 1);
        assert!(!after.contains(&taken));
    }
}
