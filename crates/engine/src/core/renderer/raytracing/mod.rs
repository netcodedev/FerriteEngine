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

use super::{shader::Shader, taa};
use crate::core::gfx::{self, BlasGeometry, Format, SamplerDesc, TextureDesc, TlasInstance};

/// Texture unit the lit shaders read the shadows from.
pub const RT_SHADOW_UNIT: u32 = 25;
/// Slot of the scene's top level structure in the trace program.
const SCENE_SLOT: u32 = 0;
/// Frames a bottom level structure is kept without being used.
const KEEP_FRAMES: u64 = 120;
/// Instance masks: opaque meshes, and cut-outs (leaves).
pub const MASK_OPAQUE: u8 = 1;
pub const MASK_CUTOUT: u8 = 2;
/// Share of the light a leaf card (any cut-out) stops, as the texture is
/// not looked at: about the share of a card that is leaf.
const CUTOUT_COVERAGE: f32 = 0.55;
/// Angular radius of the sun's disc (radians); a little larger than the
/// real one (0.0047), for shadows that soften visibly with distance.
const SUN_RADIUS: f32 = 0.008;
/// How far rays look for something in the way (metres).
const MAX_DISTANCE: f32 = 4000.0;
/// Share of this frame in the denoised result where the history is fine.
const CURRENT_WEIGHT: f32 = 0.2;

thread_local! {
    static COLLECTING: Cell<bool> = const { Cell::new(false) };
    static FRAME: Cell<u64> = const { Cell::new(0) };
    static INSTANCES: RefCell<Vec<TlasInstance>> = const { RefCell::new(Vec::new()) };
    static CACHE: RefCell<BlasCache> = RefCell::new(BlasCache::default());
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
    COLLECTING.with(|c| c.set(true));
}

/// Stop collecting; the instances collected.
pub fn end_collect() -> Vec<TlasInstance> {
    COLLECTING.with(|c| c.set(false));
    let frame = FRAME.with(Cell::get);
    CACHE.with(|c| c.borrow_mut().sweep(frame));
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

/// A mesh placed at `transform` (object to world).
pub fn add_mesh(geometry: BlasGeometry, transform: &Matrix4<f32>) {
    add_instances(geometry, std::iter::once(*transform));
}

/// A mesh placed at each of `transforms`.
pub fn add_instances(geometry: BlasGeometry, transforms: impl IntoIterator<Item = Matrix4<f32>>) {
    if !collecting() || geometry.count < 3 {
        return;
    }
    let frame = FRAME.with(Cell::get);
    let Some(blas) = CACHE.with(|c| c.borrow_mut().get(&[geometry], frame)) else {
        return;
    };
    let mask = mask(geometry.opaque);
    INSTANCES.with(|instances| {
        let mut instances = instances.borrow_mut();
        for transform in transforms {
            let columns: [[f32; 4]; 4] = transform.into();
            instances.push(TlasInstance {
                transform: TlasInstance::rows(&columns),
                blas,
                mask,
                custom: 0,
            });
        }
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
        let (raw, raw_target) = image(Format::R8);
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

    /// Build this frame's top level structure from `instances`.
    pub fn build(&mut self, instances: &[TlasInstance]) {
        self.instance_count = instances.len();
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
}
