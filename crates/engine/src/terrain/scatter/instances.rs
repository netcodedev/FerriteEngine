//! One chunk's placements of a layer and drawing them instanced.

use std::cell::RefCell;

use cgmath::{EuclideanSpace, InnerSpace, Matrix4, Point3, Quaternion, Rad, Rotation3, Vector3, Zero};
use glfw::WindowEvent;
use rapier3d::prelude::{ColliderBuilder, ColliderHandle, Pose, Rotation, Vector};

use crate::core::{
    entity::{component::Component, Entity},
    gfx,
    physics::physics_engine::{collider_tag, PhysicsEngine},
    material::{wind, Material, Pipeline},
    renderer::{depth_pass, pbr::Lighting, shader::Shader},
    scene::Scene,
    view_frustum::{Culled, ViewFrustum},
};

use super::super::ChunkBounds;

use super::*;
use super::{
    detail::CellKey,
    graph::{Placement, ScatterRange},
};

/// An instance taken out of a `ScatterInstances`.
pub struct TakenInstance {
    pub spec: ScatterSpec,
    pub variant: usize,
    /// Where it stood (its base).
    pub position: Point3<f32>,
    pub yaw: f32,
}

/// One layer's instances on one chunk.
pub struct ScatterInstances {
    spec: ScatterSpec,
    /// `instances_by_variant[k]` — placements assigned to variant `k`. Empty slots
    /// are preserved so indices align with `ScatterModel::variants`.
    instances_by_variant: Vec<Vec<Matrix4<f32>>>,
    /// `cells_by_variant[k]` — runs of `instances_by_variant[k]` (sorted
    /// by cell) standing in the same `CELL_SIZE` square, culled and given
    /// a detail tier together. Empty for `single`, which draws everything.
    cells_by_variant: Vec<Vec<InstanceCell>>,
    center: Point3<f32>,
    /// Box around every instance, for frustum culling.
    bounds: ChunkBounds,
    /// Each instance, in the order of `colliders`.
    placements: Vec<Placement>,
    /// Colliders these instances added, so they can be taken out again
    /// without removing the chunk they stand on.
    colliders: Vec<ColliderHandle>,
    /// The detail cell these are (see `detail`); `None` for landscape
    /// instances.
    cell: Option<CellKey>,
    gpu: RefCell<Option<ScatterGpu>>,
}

/// What the lighting uniforms of a scatter program were last set for:
/// the program, the pass (see `ScatterInstances::new_pass`), its view and
/// how it draws. Every chunk's instances of every layer share a program,
/// and the light is the same for all of them within a pass.
#[derive(Clone, Copy, PartialEq)]
pub(super) struct LitFor {
    pub(super) program: u32,
    pub(super) pass: u64,
    pub(super) view_projection: [f32; 16],
    pub(super) to_view: bool,
    pub(super) linear: bool,
    pub(super) ssao_depth: bool,
}

thread_local! {
    static PASS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static LIT_FOR: RefCell<Vec<LitFor>> = const { RefCell::new(Vec::new()) };
}

/// The pass counted by `ScatterInstances::new_pass`.
pub(super) fn current_pass() -> u64 {
    PASS.with(|pass| pass.get())
}

/// Whether the lighting has to be set for `lit_for` (`None`: always, as
/// for a preview, and the next pass sets it again): not if it was set
/// for the same already. Noted as set from here on.
pub(super) fn lighting_needed(lit_for: Option<LitFor>) -> bool {
    LIT_FOR.with(|lit| {
        let mut lit = lit.borrow_mut();
        let Some(lit_for) = lit_for else {
            // Set without a pass: the next pass sets it again.
            lit.clear();
            return true;
        };
        match lit.iter_mut().find(|l| l.program == lit_for.program) {
            Some(l) if *l == lit_for => false,
            Some(l) => {
                *l = lit_for;
                true
            }
            None => {
                lit.push(lit_for);
                true
            }
        }
    })
}

/// Set the wind's time for this frame (the terrain does, once a frame).
pub fn set_wind_clock(seconds: f32) {
    wind::set_clock(seconds);
}

/// Edge of the squares instances are culled and tiered by, in metres.
const CELL_SIZE: f32 = 16.0;

/// A run of instances in the same square (see `cells_by_variant`).
#[derive(Clone, Copy, Debug)]
pub(super) struct InstanceCell {
    pub(super) start: u32,
    pub(super) count: u32,
    /// Sphere around everything the instances draw.
    pub(super) center: Point3<f32>,
    pub(super) radius: f32,
}

/// Sort `instances` by the `CELL_SIZE` square they stand in and return the
/// runs. `reach`, `low`, `high`: how far a variant extends sideways, down
/// and up from where it stands.
pub(super) fn group_into_cells(
    instances: &mut [Matrix4<f32>],
    reach: f32,
    low: f32,
    high: f32,
) -> Vec<InstanceCell> {
    let cell_of = |m: &Matrix4<f32>| {
        (
            (m.w.x / CELL_SIZE).floor() as i32,
            (m.w.z / CELL_SIZE).floor() as i32,
        )
    };
    instances.sort_by_key(cell_of);
    let mut cells = Vec::new();
    let mut start = 0;
    while start < instances.len() {
        let key = cell_of(&instances[start]);
        let mut end = start + 1;
        while end < instances.len() && cell_of(&instances[end]) == key {
            end += 1;
        }
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        for m in &instances[start..end] {
            let (x, y, z) = (m.w.x, m.w.y, m.w.z);
            min = [min[0].min(x - reach), min[1].min(y + low), min[2].min(z - reach)];
            max = [max[0].max(x + reach), max[1].max(y + high), max[2].max(z + reach)];
        }
        let half = Vector3::new(max[0] - min[0], max[1] - min[1], max[2] - min[2]) * 0.5;
        cells.push(InstanceCell {
            start: start as u32,
            count: (end - start) as u32,
            center: Point3::new(min[0] + half.x, min[1] + half.y, min[2] + half.z),
            radius: (half.x * half.x + half.y * half.y + half.z * half.z).sqrt(),
        });
        start = end;
    }
    cells
}

struct ScatterGpu {
    variants: Vec<Option<VariantGpu>>,
}

struct VariantGpu {
    /// The instances' transforms (`instance_layout`).
    instances: gfx::Buffer,
    instance_count: u32,
}

impl ScatterInstances {
    pub fn new(spec: &ScatterSpec, placements: &[Placement]) -> Self {
        let spec = spec.clone();
        if placements.is_empty() {
            return Self {
                spec,
                instances_by_variant: Vec::new(),
                cells_by_variant: Vec::new(),
                center: Point3::new(0.0, 0.0, 0.0),
                bounds: ChunkBounds {
                    min: (0, 0, 0),
                    max: (0, 0, 0),
                },
                placements: Vec::new(),
                colliders: Vec::new(),
                cell: None,
                gpu: RefCell::new(None),
            };
        }
        let variant_count = spec.model.variants.len();
        let mut instances_by_variant: Vec<Vec<Matrix4<f32>>> =
            (0..variant_count).map(|_| Vec::new()).collect();

        for p in placements {
            let variant_idx = pick_variant_index(p.position[0], p.position[2], variant_count);
            instances_by_variant[variant_idx].push(p.matrix());
        }

        let sum: Vector3<f32> = placements
            .iter()
            .fold(Vector3::zero(), |acc, p| acc + Vector3::from(p.position));
        let center = Point3::from_vec(sum / placements.len() as f32);

        // Every variant's box, turned any way around its upright axis, as
        // large as the largest instance; a leaning one may reach out as far
        // as it is tall.
        let (mut reach, mut low, mut high) = (0.0f32, 0.0f32, 0.0f32);
        for variant in &spec.model.variants {
            let (min, max) = variant.bounds;
            reach = reach.max(min[0].abs()).max(max[0].abs()).max(min[2].abs()).max(max[2].abs());
            low = low.min(min[1]);
            high = high.max(max[1]);
        }
        let largest = placements.iter().flat_map(|p| p.scale).fold(1.0f32, f32::max);
        let leaning = placements.iter().any(|p| p.up != [0.0, 1.0, 0.0]);
        (reach, low, high) = (reach * largest, low * largest, high * largest);
        if leaning {
            reach = reach.max(high).max(-low);
        }
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        for &Placement { position: [x, y, z], .. } in placements {
            min = [min[0].min(x - reach), min[1].min(y + low), min[2].min(z - reach)];
            max = [max[0].max(x + reach), max[1].max(y + high), max[2].max(z + reach)];
        }
        let bounds = ChunkBounds {
            min: (min[0].floor() as i32, min[1].floor() as i32, min[2].floor() as i32),
            max: (max[0].ceil() as i32, max[1].ceil() as i32, max[2].ceil() as i32),
        };
        let cells_by_variant = instances_by_variant
            .iter_mut()
            .map(|instances| group_into_cells(instances, reach, low, high))
            .collect();

        Self {
            spec,
            instances_by_variant,
            cells_by_variant,
            center,
            bounds,
            placements: placements.to_vec(),
            colliders: Vec::new(),
            cell: None,
            gpu: RefCell::new(None),
        }
    }

    /// A detail output's instances on one cell (no colliders).
    pub fn new_detail(spec: &ScatterSpec, cell: CellKey, placements: &[Placement]) -> Self {
        let mut instances = Self::new(spec, placements);
        instances.cell = Some(cell);
        instances
    }

    /// The detail cell these instances are, if they are a detail output's.
    pub fn detail_cell(&self) -> Option<CellKey> {
        self.cell
    }

    pub fn spec(&self) -> &ScatterSpec {
        &self.spec
    }

    /// How many instances there are.
    pub fn len(&self) -> usize {
        self.instances_by_variant.iter().map(Vec::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.instances_by_variant.iter().all(|v| v.is_empty())
    }

    /// One instance of `variant` at the origin, turned by `yaw`; place it
    /// with the parent transform when drawing.
    pub fn single(spec: &ScatterSpec, variant: usize, yaw: f32) -> Self {
        let mut instances_by_variant: Vec<Vec<Matrix4<f32>>> =
            (0..spec.model.variants.len()).map(|_| Vec::new()).collect();
        if let Some(slot) = instances_by_variant.get_mut(variant) {
            slot.push(Quaternion::from_angle_y(Rad(yaw)).into());
        }
        Self {
            spec: spec.clone(),
            instances_by_variant,
            cells_by_variant: Vec::new(),
            center: Point3::new(0.0, 0.0, 0.0),
            bounds: ChunkBounds {
                min: (0, 0, 0),
                max: (0, 0, 0),
            },
            placements: Vec::new(),
            colliders: Vec::new(),
            cell: None,
            gpu: RefCell::new(None),
        }
    }

    /// Take out the instance whose collider is `collider` (and the
    /// collider), e.g. a tree that was felled. `None` if it is not one of
    /// these instances.
    pub fn take_instance(
        &mut self,
        collider: ColliderHandle,
        physics: &mut PhysicsEngine,
    ) -> Option<TakenInstance> {
        let index = self.colliders.iter().position(|c| *c == collider)?;
        physics.remove_collider(self.colliders.remove(index));
        let Placement { position: [x, y, z], yaw, .. } = self.placements.remove(index);
        let variant_count = self.spec.model.variants.len();
        let variant = pick_variant_index(x, z, variant_count);
        // Rebuilt from what is left; the GPU buffers follow on the next draw.
        let rest = Self::new(&self.spec, &self.placements);
        self.instances_by_variant = rest.instances_by_variant;
        self.cells_by_variant = rest.cells_by_variant;
        *self.gpu.borrow_mut() = None;
        Some(TakenInstance {
            spec: self.spec.clone(),
            variant,
            position: Point3::new(x, y, z),
            yaw,
        })
    }

    /// The collider of the instance of the layer with `salt` standing at
    /// `position` (within a few centimetres), if it is one of these.
    pub fn collider_at(&self, salt: u64, position: Point3<f32>) -> Option<ColliderHandle> {
        if self.spec.salt != salt {
            return None;
        }
        let index = self.placements.iter().position(|&Placement { position: [x, y, z], .. }| {
            (x - position.x).abs() < 0.05
                && (y - position.y).abs() < 0.05
                && (z - position.z).abs() < 0.05
        })?;
        self.colliders.get(index).copied()
    }

    /// The layer and base position of the instance whose collider is
    /// `collider`, if it is one of these.
    pub fn instance_of(&self, collider: ColliderHandle) -> Option<(u64, Point3<f32>)> {
        let index = self.colliders.iter().position(|c| *c == collider)?;
        let [x, y, z] = self.placements[index].position;
        Some((self.spec.salt, Point3::new(x, y, z)))
    }

    /// Height of the tallest part of `variant`, from its base.
    pub fn variant_height(spec: &ScatterSpec, variant: usize) -> f32 {
        spec.model
            .variants
            .get(variant)
            .map_or(1.0, |v| v.bounds.1[1].max(0.1))
    }

    /// Whether these instances are exactly what `spec` asks for.
    pub fn matches(&self, spec: &ScatterSpec) -> bool {
        self.spec.same_as(spec)
    }

    /// `parent` is the chunk's fixed body; parenting the colliders to it lets
    /// them be removed together with the chunk.
    pub fn add_colliders(
        &mut self,
        placements: &[Placement],
        scene: &mut Scene,
        parent: Option<rapier3d::prelude::RigidBodyHandle>,
    ) {
        // The colliders stand and lean as the models do, at the model's own
        // size (a scaled instance is not scaled here).
        let variant_count = self.spec.model.variants.len();
        for p in placements {
            let [x, y, z] = p.position;
            let variant_idx = pick_variant_index(x, z, variant_count);
            let translation = Vector::new(x, y, z);
            let q = p.rotation();
            let rotation = Rotation::from_xyzw(q.v.x, q.v.y, q.v.z, q.s);
            let pose = Pose::from_parts(translation, rotation);
            let collider = ColliderBuilder::new(self.spec.model.variants[variant_idx].collider_shape.clone())
                .position(pose)
                .user_data(collider_tag::SCATTER)
                .build();
            self.colliders
                .push(scene.physics_engine.add_collider(collider, parent));
        }
    }

    /// Take these instances' colliders out of the physics world. Harmless
    /// if they are already gone with the chunk body.
    pub fn remove_colliders(&mut self, physics: &mut PhysicsEngine) {
        for handle in self.colliders.drain(..) {
            physics.remove_collider(handle);
        }
    }

    fn ensure_gpu(&self) {
        if self.gpu.borrow().is_some() || crate::core::headless::is_headless() {
            return;
        }
        let gpu = build_instances_gpu(&self.instances_by_variant);
        *self.gpu.borrow_mut() = Some(gpu);
    }
}

fn build_instances_gpu(instances_by_variant: &[Vec<Matrix4<f32>>]) -> ScatterGpu {
    let mut kinds: Vec<Option<VariantGpu>> = Vec::with_capacity(instances_by_variant.len());
    for instances in instances_by_variant {
        if instances.is_empty() {
            kinds.push(None);
            continue;
        }

        // Per-variant instance buffer: each transform's columns.
        let mut buf: Vec<f32> = Vec::with_capacity(instances.len() * 16);
        for m in instances {
            let cols: [[f32; 4]; 4] = (*m).into();
            for c in &cols {
                buf.extend_from_slice(c);
            }
        }
        kinds.push(Some(VariantGpu {
            instances: gfx::Buffer::from_slice(gfx::BufferUsage::VERTEX, gfx::BufferMemory::Static, &buf),
            instance_count: instances.len() as u32,
        }));
    }
    ScatterGpu { variants: kinds }
}

/// The tier a shadow map draws at `distance`: one coarser than the view's,
/// but not the last of more than two (typically a billboard, whose shadow
/// would not look like the tree's) unless the view uses it too.
pub(super) fn shadow_lod_index(distance: f32, lod_count: usize) -> usize {
    let view = pick_lod_index(distance, lod_count);
    let coarsest = if lod_count > 2 { lod_count - 2 } else { lod_count.saturating_sub(1) };
    view.max((view + 1).min(coarsest))
}

fn pick_lod_index(distance: f32, lod_count: usize) -> usize {
    if lod_count == 0 {
        return 0;
    }
    // The graphics settings move the distances out or in.
    let distance = distance / crate::core::graphics::engine().scatter_lod_factor;
    let max_lod = lod_count - 1;
    if distance < LOD_DIST_NEAR {
        return 0;
    }
    if distance < LOD_DIST_MID {
        return 1.min(max_lod);
    }
    if distance < LOD_DIST_FAR {
        return 2.min(max_lod);
    }
    3.min(max_lod)
}

impl Component for ScatterInstances {
    fn update(&mut self, _: &mut Scene, _: &mut Entity, _: f64) {}

    fn render(
        &self,
        scene: &Scene,
        _: &Entity,
        view_projection: &Matrix4<f32>,
        parent_transform: &Matrix4<f32>,
    ) {
        // The ground map holds the terrain alone.
        if crate::core::renderer::weather::exposure::in_ground_pass() {
            return;
        }
        if !ViewFrustum::is_bounds_in_frustum_matrix(view_projection, self.bounds) {
            let cells: usize = self.cells_by_variant.iter().map(Vec::len).sum();
            ViewFrustum::count_many(Culled::ScatterCell, cells as u32, 0);
            return;
        }
        let Some(lighting) = Lighting::from_scene(scene) else {
            return;
        };
        // Shadow maps and other depth passes away from the camera's view
        // take a coarser tier: their texels are bigger than the detail.
        let in_view = crate::core::renderer::hdr::draws_to_view()
            || crate::core::renderer::ssao::in_depth_pass();
        // Collecting for ray tracing decides for itself (grass may cast
        // ray traced shadows).
        if !in_view && !self.spec.shadows && !crate::core::renderer::raytracing::collecting() {
            return;
        }
        let lod = LodChoice::PerCell {
            eye: lighting.camera_position,
            shadow: !in_view,
        };
        self.draw_lit(view_projection, parent_transform, &lighting, lod, true);
    }

    fn handle_event(&mut self, _: &mut glfw::Glfw, _: &mut glfw::Window, _: &WindowEvent) {}

    fn on_remove(&mut self, physics: &mut PhysicsEngine) {
        self.remove_colliders(physics);
    }
}

/// How far from the camera grass casts ray traced shadows (metres).
const RAY_TRACED_GRASS_DISTANCE: f32 = 40.0;

impl ScatterInstances {
    /// A new pass begins (the terrain calls this before its chunks are
    /// drawn): the lighting is set on the scatter program again, once.
    pub fn new_pass() {
        PASS.with(|pass| pass.set(pass.get().wrapping_add(1)));
    }

    /// Draw the instances. Needs no scene, so previews can use it too.
    pub fn draw(
        &self,
        view_projection: &Matrix4<f32>,
        parent_transform: &Matrix4<f32>,
        lighting: &Lighting,
        lod: LodChoice,
    ) {
        self.draw_lit(view_projection, parent_transform, lighting, lod, false);
    }

    /// Hand the instances to the ray traced shadows' scene: landscape
    /// instances that cast shadows (trees, rocks), each cell at the detail
    /// tier its shadow would take. Detail scatter (grass) only with its own
    /// setting, and only near the camera: many instances for what its
    /// shadows add; then it casts them whatever its layer says (its shadow
    /// maps would have been too coarse for it).
    fn collect_for_ray_tracing(&self, view_projection: &Matrix4<f32>, parent_transform: &Matrix4<f32>, eye: Point3<f32>) {
        use crate::core::renderer::raytracing;
        let grass = match self.spec.range {
            ScatterRange::Landscape if self.spec.shadows => false,
            ScatterRange::Detail { .. } if crate::core::graphics::engine().ray_traced_grass => true,
            _ => return,
        };
        self.ensure_gpu();
        let gpu_borrow = self.gpu.borrow();
        let gpu = gpu_borrow.as_ref().expect("gpu just initialized");
        let r = &*self.spec.model;
        let planes = ViewFrustum::planes(view_projection);
        for (variant_idx, variant_gpu) in gpu.variants.iter().enumerate() {
            if variant_gpu.is_none() {
                continue;
            }
            let kind = &r.variants[variant_idx];
            let tiers = kind.lods.len();
            if tiers == 0 {
                continue;
            }
            let Some(placed) = self.instances_by_variant.get(variant_idx) else {
                continue;
            };
            // (tier, first instance, instances).
            let runs: Vec<(usize, usize, usize)> = match self.cells_by_variant.get(variant_idx) {
                Some(cells) if !cells.is_empty() => cells
                    .iter()
                    .filter(|cell| ViewFrustum::is_sphere_visible(&planes, cell.center, cell.radius))
                    .filter(|cell| !grass || (cell.center - eye).magnitude() - cell.radius < RAY_TRACED_GRASS_DISTANCE)
                    .map(|cell| {
                        let tier = shadow_lod_index((cell.center - eye).magnitude(), tiers);
                        (tier, cell.start as usize, cell.count as usize)
                    })
                    .collect(),
                _ if grass && (self.center - eye).magnitude() > RAY_TRACED_GRASS_DISTANCE => Vec::new(),
                _ => vec![(shadow_lod_index((self.center - eye).magnitude(), tiers), 0, placed.len())],
            };
            for (tier, start, count) in runs {
                let Some(transforms) = placed.get(start..start + count) else {
                    continue;
                };
                for sub in &kind.lods[tier].submeshes {
                    let material = kind.model.material(sub.material_index);
                    let (shadows, opaque) = match material {
                        Some(material) => (material.shadows, material.alpha == crate::core::material::AlphaMode::Opaque),
                        None => (true, true),
                    };
                    if !shadows && !grass {
                        continue;
                    }
                    let stride = vertex_layout().stride;
                    let geometry = gfx::BlasGeometry {
                        vertices: sub.vertices.raw(),
                        stride,
                        vertex_count: (sub.vertices.size() / stride as usize) as u32,
                        indices: Some(sub.indices.raw()),
                        count: sub.index_count,
                        opaque,
                    };
                    // The UV follows position and normal (`vertex_layout`).
                    let cutout = material.and_then(|material| raytracing::Cutout::of(material, 24));
                    raytracing::add_instances(geometry, cutout, transforms.iter().map(|t| parent_transform * t));
                }
            }
        }
    }

    /// `draw`; with `in_pass`, `lighting` is the scene's for this pass and
    /// is set on the program only if it was not yet in this pass.
    fn draw_lit(
        &self,
        view_projection: &Matrix4<f32>,
        parent_transform: &Matrix4<f32>,
        lighting: &Lighting,
        lod: LodChoice,
        in_pass: bool,
    ) {
        if self.is_empty() {
            return;
        }
        if crate::core::renderer::raytracing::collecting() {
            self.collect_for_ray_tracing(view_projection, parent_transform, lighting.camera_position);
            return;
        }
        self.ensure_gpu();
        let gpu_borrow = self.gpu.borrow();
        let gpu = gpu_borrow.as_ref().expect("gpu just initialized");

        let r = &*self.spec.model;
        // Depth alone in depth passes: the cut-out and the wind, no light.
        let depth_only = depth_pass::is_active();
        let pipeline = if depth_only { Pipeline::InstancedDepth } else { Pipeline::Instanced };
        let shadow_pass = !crate::core::renderer::hdr::draws_to_view()
            && !crate::core::renderer::ssao::in_depth_pass();
        // LEQUAL: the scene's depth may already be drawn.
        let desc = gfx::PipelineDesc {
            vertex_layouts: crate::core::gfx::Layouts::of(&[vertex_layout(), instance_layout()]),
            depth: gfx::DepthState::LEQUAL,
            ..Default::default()
        };
        // Detail instances shrink away towards the edge of their range.
        let range = match self.spec.range {
            ScatterRange::Detail { distance } => distance,
            ScatterRange::Landscape => 0.0,
        };
        let eye = lighting.camera_position;
        // Set up the program a submesh's material needs, unless it is the
        // one set up last (the materials of a model mostly share one).
        let mut current: Option<u32> = None;
        let mut use_program = |shader: &Shader| {
            if current == Some(shader.id()) {
                return;
            }
            current = Some(shader.id());
            gfx::set_pipeline(shader.pipeline(&desc));
            shader.set_uniform_mat4("viewProjection", view_projection);
            shader.set_uniform_mat4("parentTransform", parent_transform);
            shader.set_uniform_4f("detailFade", eye.x, eye.z, range, self.spec.fade);
            shader.set_uniform_1i(
                "fadeThin",
                matches!(self.spec.fade_mode, super::graph::FadeMode::Thin) as i32,
            );
            wind::apply(shader, self.spec.wind);
            if depth_only {
                shader.set_uniform_3f("cameraPosition", eye.x, eye.y, eye.z);
            } else {
                shader.set_uniform_1f("normalsUp", self.spec.normals_up);
                let lit_for = in_pass.then(|| LitFor {
                    program: shader.id(),
                    pass: current_pass(),
                    view_projection: *AsRef::<[f32; 16]>::as_ref(view_projection),
                    to_view: crate::core::renderer::hdr::draws_to_view(),
                    linear: crate::core::renderer::hdr::writes_linear(),
                    ssao_depth: crate::core::renderer::ssao::in_depth_pass(),
                });
                if lighting_needed(lit_for) {
                    lighting.apply(shader);
                }
            }
        };

        let planes = matches!(lod, LodChoice::PerCell { .. })
            .then(|| ViewFrustum::planes(view_projection));
        // (tier, first instance, instances) of what is drawn.
        let mut runs: Vec<(usize, u32, u32)> = Vec::new();
        for (variant_idx, variant_gpu_opt) in gpu.variants.iter().enumerate() {
            let Some(variant_gpu) = variant_gpu_opt else { continue };
            let kind = &r.variants[variant_idx];
            let tiers = kind.lods.len();
            runs.clear();
            let cells = self.cells_by_variant.get(variant_idx);
            match (lod, &planes, cells) {
                (LodChoice::PerCell { eye, shadow }, Some(planes), Some(cells))
                    if !cells.is_empty() =>
                {
                    for cell in cells {
                        let visible = ViewFrustum::is_sphere_visible(planes, cell.center, cell.radius);
                        ViewFrustum::count(Culled::ScatterCell, visible);
                        if !visible {
                            continue;
                        }
                        let distance = (cell.center - eye).magnitude();
                        let tier = if shadow {
                            shadow_lod_index(distance, tiers)
                        } else {
                            pick_lod_index(distance, tiers)
                        };
                        // Neighbouring cells of a tier: one draw.
                        match runs.last_mut() {
                            Some((last, start, count))
                                if *last == tier && *start + *count == cell.start =>
                            {
                                *count += cell.count;
                            }
                            _ => runs.push((tier, cell.start, cell.count)),
                        }
                    }
                }
                _ => {
                    let tier = match lod {
                        LodChoice::Distance(distance) => pick_lod_index(distance, tiers),
                        LodChoice::Fixed(tier) => tier.min(tiers.saturating_sub(1)),
                        LodChoice::Shadow(distance) => shadow_lod_index(distance, tiers),
                        LodChoice::PerCell { eye, shadow } => {
                            let distance = (self.center - eye).magnitude();
                            if shadow {
                                shadow_lod_index(distance, tiers)
                            } else {
                                pick_lod_index(distance, tiers)
                            }
                        }
                    };
                    runs.push((tier, 0, variant_gpu.instance_count));
                }
            }
            for lod_idx in 0..tiers {
                if !runs.iter().any(|run| run.0 == lod_idx) {
                    continue;
                }
                let lod = &kind.lods[lod_idx];
                for sub in &lod.submeshes {
                    let fallback;
                    let material = match kind.model.material(sub.material_index) {
                        Some(material) => material,
                        None => {
                            fallback = Material::standard();
                            &fallback
                        }
                    };
                    if depth_only && shadow_pass && !material.shadows {
                        continue;
                    }
                    let shader = material.program(pipeline);
                    use_program(&shader);
                    material.bind(&shader);
                    sub.bind();
                    gfx::set_vertex_buffer(1, &variant_gpu.instances, 0, instance_layout().stride);
                    for &(_, start, count) in runs.iter().filter(|run| run.0 == lod_idx) {
                        gfx::draw_indexed_instanced(0, sub.index_count, 0, start, count);
                    }
                }
            }
        }
    }
}
