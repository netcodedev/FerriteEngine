//! Ray traced sun shadows (graphics setting `shadows.raytraced`, Vulkan
//! with ray queries only).
//!
//! Each frame, instead of the shadow cascades:
//!
//! 1. **Collect** (`begin_collect` … `end_collect`): the scene's entities
//!    are drawn once more with a view from above around the camera, as in
//!    a depth pass, into a 1×1 target. Models, terrain chunks and scatter
//!    instances do not draw then but hand their meshes over (`add_mesh`,
//!    `add_instances`): each mesh gets a bottom level acceleration
//!    structure once (kept while it is used, `BlasCache`), and each placed
//!    mesh an instance in this frame's top level structure.
//! 2. **Trace** (`RayTracedShadows::trace`, after the depth prepass): one
//!    ray per pixel from the visible surface toward a random point on the
//!    sun's disc. Cut-out materials (leaves) let part of the light through
//!    instead of being tested against their texture.
//! 3. **Denoise**: the result is blended into the frames before
//!    (reprojected with the camera, dropped where the history showed
//!    another surface), then blurred a little where the depth agrees.
//! 4. The lit shaders read the result per pixel (`sunShadow` in
//!    `pbr/lighting.glsl`) instead of the cascades.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

use cgmath::{EuclideanSpace, InnerSpace, Matrix4, Point3, SquareMatrix, Vector3};

use super::{shader::Shader, taa, texture::Texture};
use crate::core::gfx::{
    self, BlasGeometry, Format, SamplerDesc, TextureDesc, TextureKind, TlasInstance,
};

/// Texture unit the lit shaders read the shadows from.
pub const RT_SHADOW_UNIT: u32 = 25;
/// Slot of the scene's top level structure in the trace program.
const SCENE_SLOT: u32 = 0;
/// Frames a bottom level structure is kept without being used.
const KEEP_FRAMES: u64 = 120;
/// Instance masks: opaque meshes, and cut-outs (leaves).
pub const MASK_OPAQUE: u8 = 1;
pub const MASK_CUTOUT: u8 = 2;
/// Share of the light a cut-out stops whose texture the rays cannot test
/// (none found, not loaded yet, or the atlas full): about the share of a
/// leaf card that is leaf.
const CUTOUT_COVERAGE: f32 = 0.55;
/// The cut-out textures the rays test against: their opacity, one layer
/// each of an array this big.
const ATLAS_SIZE: u32 = 256;
const ATLAS_LAYERS: u32 = 64;
/// Texture unit and buffer slot of the atlas and the cut-out records in
/// the trace program.
const ATLAS_UNIT: u32 = 1;
const CUTOUT_SLOT: u32 = 5;
/// An instance's custom index without a cut-out record.
const NO_CUTOUT: u32 = 0xFF_FFFF;
/// Angular radius of the sun's disc (radians), the real one: shadows
/// soften with the distance to what casts them, as they do outside.
const SUN_RADIUS: f32 = 0.0047;
/// How far rays look for something in the way (metres).
const MAX_DISTANCE: f32 = 4000.0;
/// Share of this frame in the denoised result where the history is fine.
const CURRENT_WEIGHT: f32 = 0.2;

thread_local! {
    static COLLECTING: Cell<bool> = const { Cell::new(false) };
    static FRAME: Cell<u64> = const { Cell::new(0) };
    static INSTANCES: RefCell<Vec<TlasInstance>> = const { RefCell::new(Vec::new()) };
    static CACHE: RefCell<BlasCache> = RefCell::new(BlasCache::default());
    static CUTOUTS: RefCell<Cutouts> = RefCell::new(Cutouts::default());
    static ATLAS: RefCell<Option<OpacityAtlas>> = const { RefCell::new(None) };
    static SKINNED: RefCell<HashMap<(u64, usize), Skinned>> = RefCell::new(HashMap::new());
}

/// A skinned mesh in one pose: its posed positions and the structure
/// built from them.
struct Skinned {
    _positions: gfx::Buffer,
    blas: gfx::Blas,
    used: u64,
}

/// Frames a skinned mesh's structure is kept after its pose was last
/// seen (a pose that changed is not seen again).
const KEEP_SKINNED_FRAMES: u64 = 2;

/// A cut-out mesh's opacity texture, which rays test its triangles
/// against (alpha testing, as the drawn leaves are cut out).
pub struct Cutout<'a> {
    pub texture: &'a Texture,
    /// The texture's channel the opacity is in (0 to 3).
    pub channel: u32,
    /// Opacity below which the surface is cut away.
    pub cutoff: f32,
    /// Where in each vertex its UV (two `f32`) starts, in bytes.
    pub uv_offset: u32,
}

impl<'a> Cutout<'a> {
    /// What rays test `material` against, for vertices with their UV at
    /// byte `uv_offset`; `None` for an opaque material or one whose
    /// opacity comes from no texture.
    pub fn of(material: &'a crate::core::material::Material, uv_offset: u32) -> Option<Self> {
        let crate::core::material::AlphaMode::Cutout(cutoff) = material.alpha else {
            return None;
        };
        let (texture, channel) = material.opacity_texture()?;
        Some(Cutout {
            texture,
            channel,
            cutoff,
            uv_offset,
        })
    }
}

/// One cut-out record for the trace shader (`Cutout` in `trace.glsl`):
/// vertex and index buffer addresses, vertex stride and UV offset in
/// words, atlas layer, cut-off, whether indexed.
type CutoutRecord = [u32; 12];

/// The cut-outs of the frame being collected, and the atlas layers given
/// to opacity textures.
#[derive(Default)]
struct Cutouts {
    records: Vec<CutoutRecord>,
    by_key: HashMap<(BlasGeometry, u32, u32), u32>,
    /// Which layer of the opacity atlas (`OpacityAtlas`) holds which
    /// texture. Like the atlas, shared by every scene: a scene made later
    /// (the editor's after play mode) finds the layers drawn already.
    layers: AtlasLayers,
    /// Layers given out but not drawn yet: the texture and its channel.
    pending: Vec<(Texture, u32, u32)>,
}

/// The layers of the opacity atlas given to textures (raw id and channel),
/// with the frame each was last used in. When all are given out, the one
/// unused the longest (`KEEP_FRAMES` at least) goes to the next texture.
#[derive(Default)]
struct AtlasLayers {
    by_texture: HashMap<(gfx::RawId, u32), (u32, u64)>,
}

impl AtlasLayers {
    /// The layer of `key` in `frame`, and whether it was just given (its
    /// texture still to be drawn); `None` when every layer is in use.
    fn layer(&mut self, key: (gfx::RawId, u32), frame: u64) -> Option<(u32, bool)> {
        if let Some((layer, used)) = self.by_texture.get_mut(&key) {
            *used = frame;
            return Some((*layer, false));
        }
        let layer = if (self.by_texture.len() as u32) < ATLAS_LAYERS {
            // Layers are given in order and only taken back to be given
            // again, so the next one is free.
            self.by_texture.len() as u32
        } else {
            let (&oldest, &(layer, used)) = self
                .by_texture
                .iter()
                .min_by_key(|(_, (_, used))| *used)?;
            if frame.saturating_sub(used) <= KEEP_FRAMES {
                return None;
            }
            self.by_texture.remove(&oldest);
            layer
        };
        self.by_texture.insert(key, (layer, frame));
        Some((layer, true))
    }
}

impl Cutouts {
    /// The record index for `geometry` cut out by `cutout`, `NO_CUTOUT`
    /// when the rays cannot test it.
    fn record(&mut self, geometry: &BlasGeometry, cutout: &Cutout) -> u32 {
        let texture = cutout.texture.raw();
        if texture == 0 {
            // Still loading.
            return NO_CUTOUT;
        }
        let frame = FRAME.with(Cell::get);
        let Some((layer, new)) = self.layers.layer((texture, cutout.channel), frame) else {
            return NO_CUTOUT;
        };
        if new {
            self.pending
                .push((cutout.texture.share(), cutout.channel, layer));
        }
        let key = (*geometry, layer, cutout.cutoff.to_bits());
        if let Some(&index) = self.by_key.get(&key) {
            return index;
        }
        let vertices = gfx::buffer_address(geometry.vertices);
        let indices = geometry.indices.map_or(0, gfx::buffer_address);
        if vertices == 0 || (geometry.indices.is_some() && indices == 0) {
            return NO_CUTOUT;
        }
        let index = self.records.len() as u32;
        self.records.push([
            vertices as u32,
            (vertices >> 32) as u32,
            indices as u32,
            (indices >> 32) as u32,
            geometry.stride / 4,
            cutout.uv_offset / 4,
            layer,
            cutout.cutoff.to_bits(),
            geometry.indices.is_some() as u32,
            0,
            0,
            0,
        ]);
        self.by_key.insert(key, index);
        index
    }
}

/// Bottom level structures by the meshes they were built from.
#[derive(Default)]
struct BlasCache {
    built: HashMap<Vec<BlasGeometry>, (Rc<gfx::Blas>, u64)>,
}

impl BlasCache {
    fn get(&mut self, geometries: &[BlasGeometry], frame: u64) -> Option<gfx::RawId> {
        if let Some((blas, used)) = self.built.get_mut(geometries) {
            *used = frame;
            return Some(blas.raw());
        }
        let blas = gfx::Blas::new(geometries)?;
        let raw = blas.raw();
        self.built
            .insert(geometries.to_vec(), (Rc::new(blas), frame));
        Some(raw)
    }

    /// Drop what was not used for a while (its mesh is likely gone; buffer
    /// ids are never reused, so a mesh made later builds its own).
    fn sweep(&mut self, frame: u64) {
        self.built
            .retain(|_, (_, used)| frame.saturating_sub(*used) <= KEEP_FRAMES);
    }

    fn len(&self) -> usize {
        self.built.len()
    }
}

/// Whether meshes are being collected for ray tracing now: the drawing
/// code hands them over (`add_mesh`, `add_instances`) instead of drawing.
pub fn collecting() -> bool {
    COLLECTING.with(Cell::get)
}

/// Start collecting this frame's instances.
pub fn begin_collect() {
    FRAME.with(|f| f.set(f.get() + 1));
    INSTANCES.with(|i| i.borrow_mut().clear());
    CUTOUTS.with(|c| {
        let mut c = c.borrow_mut();
        c.records.clear();
        c.by_key.clear();
    });
    COLLECTING.with(|c| c.set(true));
}

/// Stop collecting; the instances collected.
pub fn end_collect() -> Vec<TlasInstance> {
    COLLECTING.with(|c| c.set(false));
    let frame = FRAME.with(Cell::get);
    CACHE.with(|c| c.borrow_mut().sweep(frame));
    SKINNED.with(|s| s.borrow_mut().retain(|_, skinned| frame - skinned.used <= KEEP_SKINNED_FRAMES));
    INSTANCES.with(|i| std::mem::take(&mut *i.borrow_mut()))
}

/// Bottom level structures kept now (statistics).
pub fn cached_meshes() -> usize {
    CACHE.with(|c| c.borrow().len())
}

fn mask(opaque: bool) -> u8 {
    if opaque {
        MASK_OPAQUE
    } else {
        MASK_CUTOUT
    }
}

/// A mesh placed at `transform` (object to world); `cutout` for a mesh
/// that is not opaque.
pub fn add_mesh(geometry: BlasGeometry, cutout: Option<Cutout>, transform: &Matrix4<f32>) {
    add_instances(geometry, cutout, std::iter::once(*transform));
}

/// A mesh placed at each of `transforms`.
pub fn add_instances(
    geometry: BlasGeometry,
    cutout: Option<Cutout>,
    transforms: impl IntoIterator<Item = Matrix4<f32>>,
) {
    if !collecting() || geometry.count < 3 {
        return;
    }
    let frame = FRAME.with(Cell::get);
    let Some(blas) = CACHE.with(|c| c.borrow_mut().get(&[geometry], frame)) else {
        return;
    };
    let mask = mask(geometry.opaque);
    let custom = match (&cutout, geometry.opaque) {
        (Some(cutout), false) => CUTOUTS.with(|c| c.borrow_mut().record(&geometry, cutout)),
        _ => NO_CUTOUT,
    };
    INSTANCES.with(|instances| {
        let mut instances = instances.borrow_mut();
        for transform in transforms {
            let columns: [[f32; 4]; 4] = transform.into();
            instances.push(TlasInstance {
                transform: TlasInstance::rows(&columns),
                blas,
                mask,
                custom,
            });
        }
    });
}

/// A skinned mesh placed at `transform`, in the pose `key` names (a
/// model's pose revision and the mesh's index): `positions` (posed, in
/// the model's space) are asked for only when that pose has no structure
/// yet. `indices` and `count` as for `BlasGeometry`. Cut-outs on skinned
/// meshes (hair) let a share of the light through.
pub fn add_skinned(
    key: (u64, usize),
    positions: impl FnOnce() -> Vec<[f32; 3]>,
    indices: Option<gfx::RawId>,
    count: u32,
    opaque: bool,
    transform: &Matrix4<f32>,
) {
    if !collecting() || count < 3 {
        return;
    }
    let frame = FRAME.with(Cell::get);
    let blas = SKINNED.with(|skinned| {
        let mut skinned = skinned.borrow_mut();
        if let Some(entry) = skinned.get_mut(&key) {
            entry.used = frame;
            return Some(entry.blas.raw());
        }
        let positions = positions();
        if positions.is_empty() {
            return None;
        }
        let buffer = gfx::Buffer::from_slice(
            gfx::BufferUsage::VERTEX | gfx::BufferUsage::RAY_TRACING_INPUT,
            gfx::BufferMemory::Static,
            &positions,
        );
        let geometry = BlasGeometry {
            vertices: buffer.raw(),
            stride: 12,
            vertex_count: positions.len() as u32,
            indices,
            count,
            opaque,
        };
        let blas = gfx::Blas::new(&[geometry])?;
        let raw = blas.raw();
        skinned.insert(key, Skinned { _positions: buffer, blas, used: frame });
        Some(raw)
    });
    let Some(blas) = blas else {
        return;
    };
    let columns: [[f32; 4]; 4] = (*transform).into();
    INSTANCES.with(|instances| {
        instances.borrow_mut().push(TlasInstance {
            transform: TlasInstance::rows(&columns),
            blas,
            mask: mask(opaque),
            custom: NO_CUTOUT,
        })
    });
}

/// The view the scene is collected with: everything within `radius`
/// metres of `eye` (horizontally), from far below to far above, so the
/// drawing code's culling keeps it all.
pub fn collect_view(eye: Point3<f32>, radius: f32) -> Matrix4<f32> {
    let above = Point3::new(eye.x, eye.y + 5000.0, eye.z);
    let view = Matrix4::look_at_rh(above, eye, Vector3::unit_z());
    cgmath::ortho(-radius, radius, -radius, radius, 1.0, 10_000.0) * view
}

/// The images of the shadows, as big as the view.
struct Targets {
    width: i32,
    height: i32,
    raw: gfx::Texture,
    raw_target: gfx::RenderTarget,
    /// Denoised over time: light (r), the surface's distance (g, clip w).
    history: [gfx::Texture; 2],
    history_targets: [gfx::RenderTarget; 2],
    filtered: gfx::Texture,
    filtered_target: gfx::RenderTarget,
    /// A 1×1 depth target the collecting pass "draws" into.
    _dummy: gfx::Texture,
    dummy_target: gfx::RenderTarget,
}

impl Targets {
    fn new(width: i32, height: i32) -> Self {
        let (w, h) = (width as u32, height as u32);
        let image = |format| {
            let texture = gfx::Texture::new(TextureDesc::d2(format, w, h));
            let target = gfx::RenderTarget::new(&[texture.attachment(0, None)], None);
            (texture, target)
        };
        // Light and the distance to what is in the way.
        let (raw, raw_target) = image(Format::RG16F);
        let (h0, t0) = image(Format::RG16F);
        let (h1, t1) = image(Format::RG16F);
        let (filtered, filtered_target) = image(Format::R8);
        let dummy = gfx::Texture::new(TextureDesc::d2(Format::Depth24Stencil8, 1, 1));
        let dummy_target = gfx::RenderTarget::new(&[], Some(dummy.attachment(0, None)));
        Self {
            width,
            height,
            raw,
            raw_target,
            history: [h0, h1],
            history_targets: [t0, t1],
            filtered,
            filtered_target,
            _dummy: dummy,
            dummy_target,
        }
    }
}

/// What the trace is given about the view.
pub struct ShadowView {
    /// What the view was drawn with (with TAA's jitter, if any).
    pub view_projection: Matrix4<f32>,
    pub eye: Point3<f32>,
    /// Toward the light that casts the shadows (the sun, or the moon at
    /// night).
    pub to_light: Vector3<f32>,
    /// The opaque scene's depth from the camera (the depth prepass).
    pub depth: gfx::RawId,
    pub width: i32,
    pub height: i32,
}

pub struct RayTracedShadows {
    trace_shader: Shader,
    accumulate_shader: Shader,
    filter_shader: Shader,
    quad: gfx::Buffer,
    tlas: gfx::Tlas,
    targets: Option<Targets>,
    frame: u32,
    newest: usize,
    history_valid: bool,
    /// The view-projection of the frame before.
    previous: Option<Matrix4<f32>>,
    /// Instances in the top level structure last built.
    pub instance_count: usize,
    /// This frame's cut-out records.
    cutouts: gfx::Buffer,
}

/// The fullscreen strip, no depth, no blending.
fn fullscreen() -> gfx::PipelineDesc {
    gfx::PipelineDesc {
        vertex_layouts: gfx::Layouts::of(&[gfx::VertexLayout::packed(
            0,
            gfx::StepMode::Vertex,
            &[gfx::VertexFormat::F32x2],
        )]),
        topology: gfx::Topology::TriangleStrip,
        depth: gfx::DepthState::OFF,
        cull: gfx::Cull::None,
        ..Default::default()
    }
}

/// `view_projection` with the camera at the origin: maps positions
/// relative to `eye`. In f64, then inverted: NDC to camera-relative
/// positions, which keep their precision kilometres from the origin.
fn inverse_relative(view_projection: &Matrix4<f32>, eye: Point3<f32>) -> Matrix4<f32> {
    let vp: Matrix4<f64> = view_projection.cast().expect("f32 fits f64");
    let eye: Point3<f64> = eye.cast().expect("f32 fits f64");
    let relative = vp * Matrix4::from_translation(eye.to_vec());
    relative
        .invert()
        .map_or(Matrix4::identity(), |m| m.cast().expect("f64 fits f32"))
}

/// Opacity of the cut-out textures the rays test against, a layer each
/// (given out by `AtlasLayers`). One for every scene, as the layers are.
struct OpacityAtlas {
    texture: gfx::Texture,
    targets: Vec<Option<gfx::RenderTarget>>,
    shader: Shader,
}

impl OpacityAtlas {
    fn new() -> Self {
        Self {
            texture: gfx::Texture::new(TextureDesc {
                kind: TextureKind::D2Array,
                format: Format::R8,
                width: ATLAS_SIZE,
                height: ATLAS_SIZE,
                layers: ATLAS_LAYERS,
                mips: 1,
                samples: 1,
            }),
            targets: (0..ATLAS_LAYERS).map(|_| None).collect(),
            shader: Shader::new(include_str!("../hdr/vertex.glsl"), include_str!("opacity.glsl")),
        }
    }

    /// Draw `channel` of `texture` into `layer`, with the fullscreen
    /// strip `quad`.
    fn draw(&mut self, quad: &gfx::Buffer, texture: &Texture, channel: u32, layer: u32) {
        let Some(slot) = self.targets.get_mut(layer as usize) else {
            return;
        };
        let target = slot.get_or_insert_with(|| {
            gfx::RenderTarget::new(&[self.texture.attachment(0, Some(layer))], None)
        });
        gfx::begin_pass(
            target.reference(),
            (ATLAS_SIZE, ATLAS_SIZE),
            gfx::Load::Color([0.0; 4]),
            gfx::Load::Keep,
        );
        let shader = &self.shader;
        gfx::set_pipeline(shader.pipeline(&fullscreen()));
        texture.bind_to(0);
        shader.set_uniform_1i("source", 0);
        shader.set_uniform_1i("channel", channel as i32);
        gfx::set_vertex_buffer(0, quad, 0, 8);
        gfx::draw(0, 4);
        gfx::end_pass();
        gfx::unbind_texture(0, TextureKind::D2);
    }
}

impl RayTracedShadows {
    /// `None` where the device cannot trace rays.
    pub fn new() -> Option<Self> {
        if !gfx::ray_tracing_supported() {
            return None;
        }
        let vertex = include_str!("../hdr/vertex.glsl");
        let vertices: [f32; 8] = [-1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0];
        Some(Self {
            trace_shader: Shader::new(vertex, include_str!("trace.glsl")),
            accumulate_shader: Shader::new(vertex, include_str!("accumulate.glsl")),
            filter_shader: Shader::new(vertex, include_str!("filter.glsl")),
            quad: gfx::Buffer::from_slice(
                gfx::BufferUsage::VERTEX,
                gfx::BufferMemory::Static,
                &vertices,
            ),
            tlas: gfx::Tlas::new(),
            targets: None,
            frame: 0,
            newest: 0,
            history_valid: false,
            previous: None,
            instance_count: 0,
            cutouts: gfx::Buffer::new(
                gfx::BufferUsage::STORAGE,
                gfx::BufferMemory::Dynamic,
                std::mem::size_of::<CutoutRecord>(),
            ),
        })
    }

    fn draw_quad(&self) {
        gfx::set_vertex_buffer(0, &self.quad, 0, 8);
        gfx::draw(0, 4);
    }

    /// A pass on a 1×1 target for the collecting draw (what does not
    /// collect draws there, harmlessly); end it with `gfx::end_pass`.
    pub fn begin_collect_pass(&mut self) {
        let targets = self.targets.get_or_insert_with(|| Targets::new(1, 1));
        gfx::begin_pass(
            targets.dummy_target.reference(),
            (1, 1),
            gfx::Load::Keep,
            gfx::Load::Depth(1.0),
        );
    }

    /// Build this frame's top level structure from `instances`, with the
    /// cut-outs collected with them.
    pub fn build(&mut self, instances: &[TlasInstance]) {
        self.instance_count = instances.len();
        let (records, pending) = CUTOUTS.with(|c| {
            let mut c = c.borrow_mut();
            (
                std::mem::take(&mut c.records),
                std::mem::take(&mut c.pending),
            )
        });
        ATLAS.with(|atlas| {
            let mut atlas = atlas.borrow_mut();
            let atlas = atlas.get_or_insert_with(OpacityAtlas::new);
            for (texture, channel, layer) in pending {
                atlas.draw(&self.quad, &texture, channel, layer);
            }
        });
        let mut bytes: Vec<u8> = records
            .iter()
            .flatten()
            .flat_map(|w| w.to_le_bytes())
            .collect();
        if bytes.is_empty() {
            // Never empty: a zero-sized storage buffer cannot be bound.
            bytes.resize(std::mem::size_of::<CutoutRecord>(), 0);
        }
        self.cutouts.replace(&bytes);
        self.tlas.build(instances);
    }

    /// Trace and denoise the shadows of `view`, and bind them to
    /// `RT_SHADOW_UNIT` for the shading pass. The pass running before goes
    /// on after (its viewport left to the caller).
    pub fn trace(&mut self, view: &ShadowView) {
        if !self.tlas.is_built() {
            return;
        }
        if self
            .targets
            .as_ref()
            .map_or(true, |t| (t.width, t.height) != (view.width, view.height))
        {
            self.targets = None;
            self.targets = Some(Targets::new(view.width.max(1), view.height.max(1)));
            self.history_valid = false;
        }
        self.frame = self.frame.wrapping_add(1);
        let targets = self.targets.as_ref().expect("just made");
        let size = (targets.width as u32, targets.height as u32);
        let depth_sampler = SamplerDesc::NEAREST_CLAMP;

        // 1. One ray per pixel.
        gfx::begin_pass(
            targets.raw_target.reference(),
            size,
            gfx::Load::Keep,
            gfx::Load::Keep,
        );
        let shader = &self.trace_shader;
        gfx::set_pipeline(shader.pipeline(&fullscreen()));
        gfx::bind_accel(SCENE_SLOT, &self.tlas);
        gfx::bind_raw_texture(0, view.depth, gfx::TextureKind::D2, &depth_sampler);
        shader.set_uniform_1i("sceneDepth", 0);
        shader.set_uniform_mat4(
            "inverseRelative",
            &inverse_relative(&view.view_projection, view.eye),
        );
        shader.set_uniform_3f("eye", view.eye.x, view.eye.y, view.eye.z);
        let l = view.to_light.normalize();
        shader.set_uniform_3f("toLight", l.x, l.y, l.z);
        shader.set_uniform_1f("lightRadius", SUN_RADIUS);
        shader.set_uniform_1f("maxDistance", MAX_DISTANCE);
        shader.set_uniform_1f("cutoutCoverage", CUTOUT_COVERAGE);
        shader.set_uniform_1i("frame", self.frame as i32);
        ATLAS.with(|atlas| {
            let mut atlas = atlas.borrow_mut();
            let atlas = atlas.get_or_insert_with(OpacityAtlas::new);
            gfx::bind_texture(ATLAS_UNIT, &atlas.texture, &SamplerDesc::LINEAR_REPEAT);
        });
        shader.set_uniform_1i("opacityAtlas", ATLAS_UNIT as i32);
        gfx::bind_storage_buffer(CUTOUT_SLOT, &self.cutouts);
        self.draw_quad();
        gfx::end_pass();

        // 2. Into the history.
        let next = 1 - self.newest;
        let previous = self.previous.unwrap_or(view.view_projection);
        gfx::begin_pass(
            targets.history_targets[next].reference(),
            size,
            gfx::Load::Keep,
            gfx::Load::Keep,
        );
        let shader = &self.accumulate_shader;
        gfx::set_pipeline(shader.pipeline(&fullscreen()));
        gfx::bind_texture(0, &targets.raw, &SamplerDesc::NEAREST_CLAMP);
        shader.set_uniform_1i("current", 0);
        gfx::bind_texture(
            1,
            &targets.history[self.newest],
            &SamplerDesc::NEAREST_CLAMP,
        );
        shader.set_uniform_1i("history", 1);
        gfx::bind_raw_texture(2, view.depth, gfx::TextureKind::D2, &depth_sampler);
        shader.set_uniform_1i("sceneDepth", 2);
        shader.set_uniform_mat4(
            "reprojection",
            &taa::reprojection(&view.view_projection, &previous),
        );
        let [x, y, z, w] = taa::inverse_w(&view.view_projection);
        shader.set_uniform_4f("inverseW", x, y, z, w);
        shader.set_uniform_1i("historyValid", self.history_valid as i32);
        shader.set_uniform_1f("currentWeight", CURRENT_WEIGHT);
        self.draw_quad();
        gfx::end_pass();
        self.newest = next;
        self.history_valid = true;
        self.previous = Some(view.view_projection);

        // 3. A little blur where the depth agrees.
        gfx::begin_pass(
            targets.filtered_target.reference(),
            size,
            gfx::Load::Keep,
            gfx::Load::Keep,
        );
        let shader = &self.filter_shader;
        gfx::set_pipeline(shader.pipeline(&fullscreen()));
        gfx::bind_texture(0, &targets.history[next], &SamplerDesc::NEAREST_CLAMP);
        shader.set_uniform_1i("accumulated", 0);
        gfx::bind_texture(1, &targets.raw, &SamplerDesc::NEAREST_CLAMP);
        shader.set_uniform_1i("occluders", 1);
        shader.set_uniform_1f("pixelScale", pixel_scale(&view.view_projection, targets.height));
        shader.set_uniform_1f("lightRadius", SUN_RADIUS);
        self.draw_quad();
        gfx::end_pass();
        for unit in 0..3 {
            gfx::unbind_texture(unit, gfx::TextureKind::D2);
        }
        gfx::bind_texture(
            RT_SHADOW_UNIT,
            &targets.filtered,
            &SamplerDesc::NEAREST_CLAMP,
        );
    }

    /// Forget the history (a cut).
    pub fn reset(&mut self) {
        self.history_valid = false;
        self.previous = None;
    }

    /// Raw id of the denoised shadows, for debug views.
    pub fn result_texture(&self) -> Option<gfx::RawId> {
        self.targets.as_ref().map(|t| t.filtered.raw())
    }
}

/// How many pixels (of a view `height` tall) a metre spans one metre in
/// front of the camera of `view_projection`: its focal length (the length
/// of the projection's y row, the view's rotation keeps it) times half the
/// height.
fn pixel_scale(view_projection: &Matrix4<f32>, height: i32) -> f32 {
    let row = Vector3::new(view_projection.x.y, view_projection.y.y, view_projection.z.y);
    row.magnitude() * height as f32 * 0.5
}

/// Whether `eye` moved so far since `last` that the history is useless.
pub fn is_cut(eye: Point3<f32>, last: Option<Point3<f32>>) -> bool {
    last.is_none_or(|last| (eye - last).magnitude() > 50.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgmath::Vector4;

    #[test]
    fn camera_relative_positions_come_back_far_from_the_origin() {
        let eye = Point3::new(12_000.0, 150.0, -9_000.0);
        let projection = cgmath::perspective(cgmath::Deg(60.0), 1.5, 0.1, 20_000.0);
        let vp = projection
            * Matrix4::look_at_rh(eye, eye + Vector3::new(1.0, -0.2, 0.3), Vector3::unit_y());
        let world = Vector4::new(12_030.0, 140.0, -8_990.0, 1.0);
        // Projected in f64: in f32 the test itself would lose centimetres.
        let vp64: Matrix4<f64> = vp.cast().unwrap();
        let clip = vp64 * world.cast::<f64>().unwrap();
        let ndc = (clip / clip.w).cast::<f32>().unwrap();
        let relative = inverse_relative(&vp, eye) * Vector4::new(ndc.x, ndc.y, ndc.z, 1.0);
        let relative = relative.truncate() / relative.w;
        let back = eye + relative;
        assert!(
            (back - Point3::new(world.x, world.y, world.z)).magnitude() < 0.05,
            "{back:?}"
        );
    }

    #[test]
    fn the_collect_view_keeps_what_is_around_the_camera() {
        let eye = Point3::new(500.0, 80.0, -200.0);
        let vp = collect_view(eye, 300.0);
        for p in [
            Point3::new(780.0, -300.0, -470.0),
            Point3::new(230.0, 2000.0, 60.0),
            eye,
        ] {
            let c = vp * p.to_homogeneous();
            let n = c.truncate() / c.w;
            assert!(
                n.x.abs() <= 1.0 && n.y.abs() <= 1.0 && n.z.abs() <= 1.0,
                "{p:?} -> {n:?}"
            );
        }
        let far = vp * Point3::new(900.0, 80.0, -200.0).to_homogeneous();
        assert!((far.x / far.w).abs() > 1.0);
    }

    #[test]
    fn a_metre_spans_the_pixels_the_field_of_view_gives() {
        let projection = cgmath::perspective(cgmath::Deg(90.0), 1.5, 0.1, 1000.0);
        let view = Matrix4::look_at_rh(Point3::new(5.0, 2.0, 1.0), Point3::new(9.0, 1.0, 3.0), Vector3::unit_y());
        // 90°: one metre at one metre spans half the height.
        assert!((pixel_scale(&(projection * view), 600) - 300.0).abs() < 0.01);
    }

    #[test]
    fn atlas_layers_are_kept_and_given_again_once_unused() {
        let mut layers = AtlasLayers::default();
        assert_eq!(layers.layer((7, 3), 1), Some((0, true)));
        assert_eq!(layers.layer((8, 3), 1), Some((1, true)));
        // Asked again (a scene made later): drawn already.
        assert_eq!(layers.layer((7, 3), 2), Some((0, false)));
        for texture in 9..ATLAS_LAYERS as u64 + 7 {
            assert!(layers.layer((texture, 0), 2).is_some_and(|(_, new)| new));
        }
        // Full, and everything used lately.
        assert_eq!(layers.layer((500, 0), 3), None);
        // Texture 8 alone is used no more: its layer goes to the next.
        let later = 3 + KEEP_FRAMES + 1;
        for texture in (9..ATLAS_LAYERS as u64 + 7).chain([7]) {
            let channel = if texture == 7 { 3 } else { 0 };
            assert!(layers.layer((texture, channel), later).is_some_and(|(_, new)| !new));
        }
        assert_eq!(layers.layer((500, 0), later), Some((1, true)));
    }
}
