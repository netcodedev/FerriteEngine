//! The render layer: GPU resources and commands, the same for every
//! backend (see docs/render-backends.md).
//!
//! The device is chosen once when the window opens (`init`) and lives on
//! the render thread; everything here is called there. Resources are
//! owned values that free their GPU objects when dropped (deferred until
//! the GPU is done with them, and safe to drop on any thread). Commands
//! are immediate functions, as GL's are: begin a pass on a target, set a
//! pipeline, bind, draw, end the pass. Passes nest: a pass begun inside
//! another (an impostor bake in the middle of the opaque pass) suspends
//! it, and ending the inner one resumes it.
//!
//! Binding slots are flat numbers, like GL texture units and buffer
//! binding points: 0–7 belong to the material, 8–15 to the pass, 16–31
//! to the frame, 32–39 to the draw. A backend with descriptor sets maps
//! the ranges onto sets 2, 1, 0 and 3.

pub mod fx;
mod gl_backend;
pub mod glsl;
mod null;
pub mod types;
#[cfg(feature = "vulkan")]
mod vulkan;

use std::{
    cell::{Cell, RefCell},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};

pub use types::*;

/// A backend's name for one of its objects. 0 is never a live object.
pub type RawId = u64;

/// Something to destroy once the GPU is done with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Raw {
    Buffer(RawId),
    Texture(RawId),
    Sampler(RawId),
    Target(RawId),
    Program(RawId),
    Pipeline(RawId),
    Queries(RawId),
    /// A ray tracing acceleration structure (bottom or top level).
    Accel(RawId),
}

/// The backend interface. Every method is called on the render thread.
pub trait Device {
    fn kind(&self) -> BackendKind;
    /// Renderer and API version (`"<renderer>, <version>"`), for diagnostics.
    fn describe(&self) -> String;
    /// Video memory `(used, total)` in bytes, where the driver tells.
    fn video_memory(&self) -> Option<(u64, u64)> {
        None
    }

    fn create_buffer(&self, usage: BufferUsage, memory: BufferMemory, size: usize, data: Option<&[u8]>) -> RawId;
    /// Replace `data.len()` bytes at `offset`. `orphan` hands the driver a
    /// fresh store first (the old contents may still be in use).
    fn write_buffer(&self, buffer: RawId, offset: usize, data: &[u8], orphan: bool);
    /// Give the buffer a new, larger store (contents lost).
    fn resize_buffer(&self, buffer: RawId, size: usize);

    fn create_texture(&self, desc: &TextureDesc) -> RawId;
    /// Upload `data` into `region` of `mip`, at layers `layer..layer + layers`
    /// (z for a 3D texture).
    fn write_texture(
        &self,
        texture: RawId,
        desc: &TextureDesc,
        mip: u32,
        region: Rect,
        layer: u32,
        layers: u32,
        format: UploadFormat,
        data: &[u8],
    );
    fn generate_mips(&self, texture: RawId, desc: &TextureDesc);
    fn create_sampler(&self, desc: &SamplerDesc) -> RawId;

    fn create_target(&self, colors: &[Attachment], depth: Option<Attachment>) -> RawId;

    /// Compile and link a program; the error text names the stage and the
    /// failing source lines.
    fn create_program(&self, vertex: &str, fragment: &str, label: &str) -> Result<RawId, String>;
    fn create_pipeline(&self, program: RawId, desc: &PipelineDesc) -> RawId;
    /// Set the loose uniform `name` (an element `name[i]` too) of
    /// `program` for the draws that follow.
    fn set_uniform(&self, program: RawId, name: &str, value: UniformValue);

    fn create_queries(&self, count: u32) -> RawId;
    fn write_timestamp(&self, queries: RawId, index: u32);
    /// The timestamps (ns) of `count` queries, if all are available yet.
    fn read_timestamps(&self, queries: RawId, count: u32) -> Option<Vec<u64>>;

    fn destroy(&self, raw: Raw);

    /// Whether the device traces rays from shaders (`GL_EXT_ray_query`)
    /// through acceleration structures. Without, the calls below do
    /// nothing and return 0.
    fn ray_tracing(&self) -> bool {
        false
    }
    /// A bottom level acceleration structure of `geometries` (built on
    /// the GPU before this frame's draws).
    fn create_blas(&self, _geometries: &[BlasGeometry]) -> RawId {
        0
    }
    /// (Re)build top level structure `tlas` (0: a new one) from
    /// `instances`, now in the frame's commands; returns its id.
    fn build_tlas(&self, _tlas: RawId, _instances: &[TlasInstance]) -> RawId {
        0
    }
    /// Bind `accel` (a top level structure) at `slot` for the draws that
    /// follow (`layout(binding = slot) uniform accelerationStructureEXT`).
    fn bind_accel(&self, _slot: u32, _accel: RawId) {}

    fn begin_pass(&self, target: TargetRef, size: (u32, u32), color: Load, depth: Load);
    /// Re-enter `target` after a nested pass ended (contents kept).
    fn resume_pass(&self, target: TargetRef, size: (u32, u32));
    fn end_pass(&self);
    fn set_viewport(&self, rect: Rect);
    fn set_scissor(&self, rect: Option<Rect>);
    fn set_pipeline(&self, pipeline: RawId, program: RawId);
    fn set_stencil_reference(&self, reference: u8);
    fn bind_texture(&self, slot: u32, texture: RawId, kind: TextureKind, sampler: RawId);
    fn unbind_texture(&self, slot: u32, kind: TextureKind);
    fn bind_uniform_buffer(&self, slot: u32, buffer: RawId, offset: usize, size: usize);
    fn bind_storage_buffer(&self, slot: u32, buffer: RawId, offset: usize, size: usize);
    fn set_vertex_buffer(&self, slot: u32, buffer: RawId, offset: usize, stride: u32);
    fn set_index_buffer(&self, buffer: RawId, format: IndexFormat);
    fn draw(&self, first_vertex: u32, vertices: u32, first_instance: u32, instances: u32);
    fn draw_indexed(&self, first_index: u32, indices: u32, base_vertex: i32, first_instance: u32, instances: u32);
    /// Clear planes of the current pass's target inside the viewport.
    fn clear(&self, planes: Planes, color: [f32; 4], depth: f32, stencil: u8);
    fn blit(&self, src: TargetRef, src_rect: Rect, dst: TargetRef, dst_rect: Rect, planes: Planes, linear: bool);
    /// RGBA8 rows of `rect` of `target`'s first colour attachment, first row first.
    fn read_pixels(&self, target: TargetRef, rect: Rect) -> Vec<u8>;
    /// `read_pixels` without waiting for the GPU: an id for `take_pixels`.
    /// Backends that cannot wait later read now.
    fn read_pixels_later(&self, target: TargetRef, rect: Rect) -> u64 {
        let pixels = self.read_pixels(target, rect);
        EARLY_READS.with(|reads| {
            let mut reads = reads.borrow_mut();
            let id = reads.0 + 1;
            reads.0 = id;
            reads.1.insert(id, pixels);
            id
        })
    }
    /// The pixels of a `read_pixels_later` once the GPU has them; `None`
    /// until then (and for an id taken already).
    fn take_pixels(&self, id: u64) -> Option<Vec<u8>> {
        EARLY_READS.with(|reads| reads.borrow_mut().1.remove(&id))
    }

    /// Samples per pixel of the window's colour (its multisampling).
    fn window_samples(&self) -> u32 {
        1
    }

    /// The frame starts (after the last one was presented).
    fn begin_frame(&self);
    /// The frame's commands are done: present the window.
    fn end_frame(&self);

    /// Whether presenting waits for the display. Backends that present
    /// themselves (not through the window's GL context) switch it here.
    fn set_vsync(&self, _on: bool) {}

    /// The window is about to close: wait for the GPU and let go of what
    /// belongs to the window.
    fn shutdown(&self) {}
}

/// One level and layer of a texture as a render target attachment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attachment {
    pub texture: RawId,
    pub desc: TextureDesc,
    pub mip: u32,
    /// The layer (or cube face × layer) drawn into; `None` for all layers
    /// of a layered texture, or a plain texture.
    pub layer: Option<u32>,
}

/// Where a pass draws: the window, or a target made of textures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TargetRef {
    Window,
    Target(RawId),
}

thread_local! {
    static DEVICE: Cell<Option<&'static dyn Device>> = const { Cell::new(None) };
    static PASSES: RefCell<Vec<(TargetRef, (u32, u32))>> = const { RefCell::new(Vec::new()) };
    static WINDOW_SIZE: Cell<(u32, u32)> = const { Cell::new((1, 1)) };
    static WINDOW_CLEAR: Cell<[f32; 4]> = const { Cell::new([0.0, 0.0, 0.0, 1.0]) };
    static VIEWPORT: Cell<Rect> = const { Cell::new(Rect { x: 0, y: 0, width: 1, height: 1 }) };
    /// The samplers made so far: few, so a list is quicker than hashing.
    static SAMPLERS: RefCell<Vec<(SamplerDesc, RawId)>> = const { RefCell::new(Vec::new()) };
}

/// GPU objects dropped since the last frame, possibly on other threads.
static RELEASED: Mutex<Vec<Raw>> = Mutex::new(Vec::new());
static INITIALISED: AtomicBool = AtomicBool::new(false);
static WIREFRAME: AtomicBool = AtomicBool::new(false);

/// Draw filled geometry as outlines (a debug view); text and other
/// `PolygonMode::Solid` pipelines stay filled.
pub fn set_wireframe(on: bool) {
    WIREFRAME.store(on, Ordering::Relaxed);
}

pub fn wireframe() -> bool {
    WIREFRAME.load(Ordering::Relaxed)
}

/// What the running pass changes about every pipeline drawn in it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PassOverrides {
    /// Only depth is drawn (shadow maps, the depth prepass): no colour
    /// writes.
    pub depth_only: bool,
    /// The depth was drawn by a prepass: what draws `LESS` draws `LEQUAL`,
    /// so the same surfaces pass again.
    pub depth_less_equal: bool,
    /// Everything passes the depth test and writes no depth (2D overlays
    /// drawn in the order given).
    pub depth_ignored: bool,
    /// Stencil clipping (the UI's nested clip rectangles).
    pub stencil: StencilMode,
}

/// How draws use the stencil buffer while a pass override says so.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum StencilMode {
    #[default]
    Off,
    /// Draw only where the stencil equals the value.
    Test(u8),
    /// Draw nothing visible: where the stencil equals `reference`, apply
    /// `op` to it (whatever the depth test says).
    Write { reference: u8, op: StencilOp },
}

thread_local! {
    static OVERRIDES: Cell<PassOverrides> = const { Cell::new(PassOverrides { depth_only: false, depth_less_equal: false, depth_ignored: false, stencil: StencilMode::Off }) };
}

pub fn pass_overrides() -> PassOverrides {
    OVERRIDES.with(|o| o.get())
}

pub fn set_pass_overrides(overrides: PassOverrides) {
    OVERRIDES.with(|o| o.set(overrides));
}

/// `desc` as drawn under the pass overrides.
pub fn resolve_pipeline(desc: &PipelineDesc) -> PipelineDesc {
    let o = pass_overrides();
    let mut desc = desc.clone();
    if o.depth_only {
        desc.color_write = false;
        desc.blend = None;
    }
    if o.depth_less_equal && desc.depth.func == CompareFunc::Less {
        desc.depth.func = CompareFunc::LessEqual;
    }
    if o.depth_ignored && desc.depth.test {
        desc.depth.func = CompareFunc::Always;
        desc.depth.write = false;
    }
    match o.stencil {
        StencilMode::Off => {}
        StencilMode::Test(_) => {
            desc.stencil = Some(StencilState {
                func: CompareFunc::Equal,
                pass: StencilOp::Keep,
                fail: StencilOp::Keep,
                depth_fail: StencilOp::Keep,
                read_mask: 0xFF,
                write_mask: 0,
            });
        }
        StencilMode::Write { op, .. } => {
            desc.color_write = false;
            desc.depth.write = false;
            desc.stencil = Some(StencilState {
                func: CompareFunc::Equal,
                pass: op,
                fail: StencilOp::Keep,
                depth_fail: op,
                read_mask: 0xFF,
                write_mask: 0xFF,
            });
        }
    }
    desc
}

/// The stencil reference the pass overrides ask for.
pub fn stencil_reference() -> u8 {
    match pass_overrides().stencil {
        StencilMode::Off => 0,
        StencilMode::Test(r) | StencilMode::Write { reference: r, .. } => r,
    }
}

/// Make `device` this thread's device. Done once, by the window.
pub fn init(device: Box<dyn Device>) {
    let device: &'static dyn Device = Box::leak(device);
    log::info!("render backend: {} ({})", device.kind().name(), device.describe());
    DEVICE.with(|d| d.set(Some(device)));
    INITIALISED.store(true, Ordering::Release);
}

/// The OpenGL device, for a window whose context is current.
pub fn opengl_device() -> Box<dyn Device> {
    Box::new(gl_backend::GlDevice::new())
}

/// The Vulkan device for a window: `extensions` are the instance
/// extensions the window system needs, `create_surface` makes the
/// window's surface from the instance (raw `VkInstance` in, raw
/// `VkSurfaceKHR` out). The window's size must be set (`set_window_size`)
/// first.
#[cfg(feature = "vulkan")]
pub fn vulkan_device(
    extensions: &[String],
    create_surface: &dyn Fn(u64) -> Result<u64, String>,
    vsync: bool,
) -> Result<Box<dyn Device>, String> {
    vulkan::create(extensions, create_surface, vsync)
}

/// Whether presenting waits for the display, where the backend presents
/// itself (Vulkan; GL's swap interval belongs to the window).
pub fn set_vsync(on: bool) {
    device().set_vsync(on);
}

/// The window closes: the device lets go of it.
pub fn shutdown() {
    device().shutdown();
}

/// This thread's device; the null device where none was set up
/// (headless, tests, loader threads).
pub fn device() -> &'static dyn Device {
    DEVICE.with(|d| d.get()).unwrap_or(&null::NULL)
}

pub fn backend() -> BackendKind {
    device().kind()
}

/// Whether a GPU draws anything at all.
pub fn is_drawing() -> bool {
    backend() != BackendKind::Null
}

/// Destroy what was dropped since the last call. The application calls
/// it once per frame.
pub fn collect_garbage() {
    let released = match RELEASED.lock() {
        Ok(mut released) if !released.is_empty() => std::mem::take(&mut *released),
        _ => return,
    };
    let device = device();
    for raw in released {
        device.destroy(raw);
    }
}

fn release(raw: Raw) {
    if !INITIALISED.load(Ordering::Acquire) {
        return;
    }
    if let Ok(mut released) = RELEASED.lock() {
        released.push(raw);
    }
}

pub fn set_window_size(width: u32, height: u32) {
    WINDOW_SIZE.with(|s| s.set((width.max(1), height.max(1))));
}

pub fn window_size() -> (u32, u32) {
    WINDOW_SIZE.with(|s| s.get())
}

/// Samples per pixel of the window (1 without multisampling).
pub fn window_samples() -> u32 {
    device().window_samples().max(1)
}

/// The colour the window was cleared to last (by `begin_window_pass`).
pub fn window_clear_color() -> [f32; 4] {
    WINDOW_CLEAR.with(|c| c.get())
}

pub fn begin_frame() {
    device().begin_frame();
}

pub fn end_frame() {
    PASSES.with(|p| p.borrow_mut().clear());
    device().end_frame();
}

// ── Buffers ────────────────────────────────────────────────────────────

/// GPU memory: vertices, indices, uniforms, storage.
pub struct Buffer {
    raw: RawId,
    size: usize,
    usage: BufferUsage,
    memory: BufferMemory,
}

impl Buffer {
    pub fn new(usage: BufferUsage, memory: BufferMemory, size: usize) -> Self {
        let raw = device().create_buffer(usage, memory, size.max(4), None);
        Buffer { raw, size: size.max(4), usage, memory }
    }

    pub fn with_data(usage: BufferUsage, memory: BufferMemory, data: &[u8]) -> Self {
        let size = data.len().max(4);
        let raw = if data.len() < 4 {
            device().create_buffer(usage, memory, size, None)
        } else {
            device().create_buffer(usage, memory, size, Some(data))
        };
        Buffer { raw, size, usage, memory }
    }

    /// `T` must be plain data (numbers, `#[repr(C)]` structs of them).
    pub fn from_slice<T>(usage: BufferUsage, memory: BufferMemory, data: &[T]) -> Self {
        Self::with_data(usage, memory, as_bytes(data))
    }

    pub fn raw(&self) -> RawId {
        self.raw
    }

    pub fn size(&self) -> usize {
        self.size
    }

    /// Write at `offset`, growing the buffer (and losing what was beyond
    /// the write) if it is too small.
    pub fn write(&mut self, offset: usize, data: &[u8]) {
        if offset + data.len() > self.size {
            let size = (offset + data.len()).next_power_of_two();
            device().resize_buffer(self.raw, size);
            self.size = size;
        }
        device().write_buffer(self.raw, offset, data, false);
    }

    /// Replace the whole contents with `data` (which may be larger: the
    /// buffer grows), handing the driver a fresh store so draws still
    /// reading the old one do not stall.
    pub fn replace(&mut self, data: &[u8]) {
        if data.len() > self.size {
            let size = data.len().next_power_of_two();
            device().resize_buffer(self.raw, size);
            self.size = size;
            device().write_buffer(self.raw, 0, data, false);
        } else {
            device().write_buffer(self.raw, 0, data, true);
        }
    }

    pub fn usage(&self) -> BufferUsage {
        self.usage
    }

    pub fn memory(&self) -> BufferMemory {
        self.memory
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        if self.raw != 0 {
            release(Raw::Buffer(self.raw));
        }
    }
}

/// The bytes of a slice of plain values (numbers, `#[repr(C)]` structs
/// of them).
pub fn as_bytes<T>(data: &[T]) -> &[u8] {
    // SAFETY: the bytes are only read; callers pass plain data.
    unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, std::mem::size_of_val(data)) }
}

// ── Textures and samplers ──────────────────────────────────────────────

pub struct Texture {
    raw: RawId,
    desc: TextureDesc,
    /// Whether dropping this frees the GPU object (`share` hands out
    /// handles that do not).
    owned: bool,
}

impl Texture {
    pub fn new(desc: TextureDesc) -> Self {
        let raw = device().create_texture(&desc);
        Texture { raw, desc, owned: true }
    }

    /// A 2D texture with its levels made from `rgba` (RGBA8 rows, first
    /// row first).
    pub fn from_rgba(width: u32, height: u32, rgba: &[u8], srgb: bool, mipmaps: bool) -> Self {
        let format = if srgb { Format::Srgba8 } else { Format::Rgba8 };
        let texture = Texture::new(
            TextureDesc::d2(format, width, height).with_mips(if mipmaps { 0 } else { 1 }),
        );
        texture.write(0, Rect::sized(width, height), 0, 1, UploadFormat::Rgba8, rgba);
        if mipmaps {
            texture.generate_mips();
        }
        texture
    }

    pub fn raw(&self) -> RawId {
        self.raw
    }

    pub fn desc(&self) -> &TextureDesc {
        &self.desc
    }

    pub fn width(&self) -> u32 {
        self.desc.width
    }

    pub fn height(&self) -> u32 {
        self.desc.height
    }

    /// Another handle on the same GPU texture; it does not free it, so
    /// this one must outlive it.
    pub fn share(&self) -> Texture {
        Texture { raw: self.raw, desc: self.desc, owned: false }
    }

    pub fn write(&self, mip: u32, region: Rect, layer: u32, layers: u32, format: UploadFormat, data: &[u8]) {
        let texels = region.width as usize * region.height as usize * layers as usize;
        if data.len() < texels * format.texel_size() {
            log::error!(
                "texture upload of {} bytes is short of {}×{}×{} texels",
                data.len(),
                region.width,
                region.height,
                layers
            );
            return;
        }
        device().write_texture(self.raw, &self.desc, mip, region, layer, layers, format, data);
    }

    pub fn generate_mips(&self) {
        if self.desc.mip_count() > 1 {
            device().generate_mips(self.raw, &self.desc);
        }
    }

    /// As a render target attachment: `mip`, and one layer or all.
    pub fn attachment(&self, mip: u32, layer: Option<u32>) -> Attachment {
        Attachment { texture: self.raw, desc: self.desc, mip, layer }
    }
}

impl Drop for Texture {
    fn drop(&mut self) {
        if self.owned && self.raw != 0 {
            release(Raw::Texture(self.raw));
        }
    }
}

/// The sampler for `desc`, made once and kept.
pub fn sampler(desc: &SamplerDesc) -> RawId {
    // Anisotropic filtering, where a sampler asks for it, is as strong as
    // the graphics settings say (`display.anisotropy`).
    let mut desc = *desc;
    if desc.anisotropy > 1 {
        desc.anisotropy = crate::core::graphics::engine().anisotropy;
    }
    let desc = &desc;
    SAMPLERS.with(|s| {
        if let Some(&(_, raw)) = s.borrow().iter().find(|(d, _)| d == desc) {
            return raw;
        }
        let raw = device().create_sampler(desc);
        s.borrow_mut().push((*desc, raw));
        raw
    })
}

// ── Render targets ─────────────────────────────────────────────────────

/// A set of attachments drawn into together.
pub struct RenderTarget {
    raw: RawId,
    size: (u32, u32),
}

impl RenderTarget {
    pub fn new(colors: &[Attachment], depth: Option<Attachment>) -> Self {
        let first = colors.first().or(depth.as_ref());
        let size = first
            .map(|a| ((a.desc.width >> a.mip).max(1), (a.desc.height >> a.mip).max(1)))
            .unwrap_or((1, 1));
        let raw = device().create_target(colors, depth);
        RenderTarget { raw, size }
    }

    pub fn reference(&self) -> TargetRef {
        TargetRef::Target(self.raw)
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    pub fn raw(&self) -> RawId {
        self.raw
    }
}

impl Drop for RenderTarget {
    fn drop(&mut self) {
        if self.raw != 0 {
            release(Raw::Target(self.raw));
        }
    }
}

// ── Ray tracing ────────────────────────────────────────────────────────

/// Whether the device can trace rays (see `Device::ray_tracing`).
pub fn ray_tracing_supported() -> bool {
    device().ray_tracing()
}

/// A bottom level acceleration structure: triangles in their own space.
pub struct Blas {
    raw: RawId,
}

impl Blas {
    /// Built from `geometries`; `None` where the device cannot trace rays.
    pub fn new(geometries: &[BlasGeometry]) -> Option<Self> {
        let raw = device().create_blas(geometries);
        (raw != 0).then_some(Blas { raw })
    }

    pub fn raw(&self) -> RawId {
        self.raw
    }
}

impl Drop for Blas {
    fn drop(&mut self) {
        if self.raw != 0 {
            release(Raw::Accel(self.raw));
        }
    }
}

/// A top level acceleration structure: instances of bottom level ones,
/// built anew each frame.
#[derive(Default)]
pub struct Tlas {
    raw: RawId,
}

impl Tlas {
    pub fn new() -> Self {
        Tlas { raw: 0 }
    }

    /// Build it from `instances` now (in the frame's commands, before
    /// what is drawn after).
    pub fn build(&mut self, instances: &[TlasInstance]) {
        self.raw = device().build_tlas(self.raw, instances);
    }

    /// Built at least once.
    pub fn is_built(&self) -> bool {
        self.raw != 0
    }

    pub fn raw(&self) -> RawId {
        self.raw
    }
}

impl Drop for Tlas {
    fn drop(&mut self) {
        if self.raw != 0 {
            release(Raw::Accel(self.raw));
        }
    }
}

/// Bind `tlas` at `slot` for the draws that follow.
pub fn bind_accel(slot: u32, tlas: &Tlas) {
    device().bind_accel(slot, tlas.raw);
}

// ── Programs and pipelines ─────────────────────────────────────────────

/// A linked program (vertex + fragment stage).
pub struct Program {
    raw: RawId,
    /// Unique among every program made (a backend may reuse raw ids).
    serial: u64,
    /// The pipelines made of it so far: a program has few, so comparing
    /// is quicker than hashing.
    pipelines: RefCell<Vec<(PipelineDesc, RawId)>>,
}

impl Program {
    /// `None` (after logging why) when it does not compile or link.
    #[track_caller]
    pub fn new(vertex: &str, fragment: &str) -> Option<Self> {
        let label = std::panic::Location::caller().to_string();
        Self::labelled(vertex, fragment, &label)
    }

    pub fn labelled(vertex: &str, fragment: &str, label: &str) -> Option<Self> {
        dump_program(vertex, fragment, label);
        match device().create_program(vertex, fragment, label) {
            Ok(raw) => Some(Program { raw, serial: next_program_serial(), pipelines: RefCell::new(Vec::new()) }),
            Err(message) => {
                report_shader_error(&message);
                None
            }
        }
    }

    /// A program that draws nothing (the null device's).
    pub fn empty() -> Self {
        Program { raw: 0, serial: next_program_serial(), pipelines: RefCell::new(Vec::new()) }
    }

    pub fn raw(&self) -> RawId {
        self.raw
    }

    /// A number no other program ever had (for caches keyed by program).
    pub fn serial(&self) -> u64 {
        self.serial
    }

    /// The pipeline of this program with `desc`, made once and kept.
    pub fn pipeline(&self, desc: &PipelineDesc) -> Pipeline {
        if self.raw == 0 {
            return Pipeline { raw: 0, program: 0 };
        }
        if let Some(&(_, raw)) = self.pipelines.borrow().iter().find(|(d, _)| d == desc) {
            return Pipeline { raw, program: self.raw };
        }
        let raw = device().create_pipeline(self.raw, desc);
        self.pipelines.borrow_mut().push((*desc, raw));
        Pipeline { raw, program: self.raw }
    }
}

fn next_program_serial() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl Drop for Program {
    fn drop(&mut self) {
        if self.raw == 0 {
            return;
        }
        for (_, pipeline) in self.pipelines.borrow_mut().drain(..) {
            release(Raw::Pipeline(pipeline));
        }
        release(Raw::Program(self.raw));
    }
}

/// A program with its fixed-function state; cheap to copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pipeline {
    raw: RawId,
    program: RawId,
}

impl Pipeline {
    pub fn is_valid(&self) -> bool {
        self.raw != 0
    }
}

/// With `FERRITE_SHADER_DUMP=<dir>`, every program's sources are written
/// there (one pair per distinct program), for checking them offline (see
/// `glsl`).
fn dump_program(vertex: &str, fragment: &str, label: &str) {
    let Ok(dir) = std::env::var("FERRITE_SHADER_DUMP") else {
        return;
    };
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (vertex, fragment).hash(&mut hasher);
    let name = format!("{:016x}", hasher.finish());
    let dir = std::path::Path::new(&dir);
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(dir.join(format!("{name}.vert")), vertex);
    let _ = std::fs::write(dir.join(format!("{name}.frag")), fragment);
    let _ = std::fs::write(dir.join(format!("{name}.label")), label);
}

/// Report a shader error once per distinct message.
pub fn report_shader_error(message: &str) {
    static REPORTED: Mutex<Vec<String>> = Mutex::new(Vec::new());
    if let Ok(mut reported) = REPORTED.lock() {
        if reported.iter().any(|m| m == message) {
            return;
        }
        reported.push(message.to_owned());
    }
    if log::log_enabled!(log::Level::Error) {
        log::error!("{message}");
    } else {
        eprintln!("[shader error] {message}");
    }
}

// ── Timestamp queries ──────────────────────────────────────────────────

pub struct Queries {
    raw: RawId,
    count: u32,
}

impl Queries {
    pub fn new(count: u32) -> Self {
        Queries { raw: device().create_queries(count), count }
    }

    pub fn timestamp(&self, index: u32) {
        if index < self.count {
            device().write_timestamp(self.raw, index);
        }
    }

    /// All timestamps (ns), once the GPU has written them.
    pub fn read(&self) -> Option<Vec<u64>> {
        device().read_timestamps(self.raw, self.count)
    }

    /// The first `count` timestamps (ns), once the GPU has written them.
    pub fn read_first(&self, count: u32) -> Option<Vec<u64>> {
        if count == 0 {
            return Some(Vec::new());
        }
        device().read_timestamps(self.raw, count.min(self.count))
    }
}

impl Drop for Queries {
    fn drop(&mut self) {
        if self.raw != 0 {
            release(Raw::Queries(self.raw));
        }
    }
}

// ── Commands ───────────────────────────────────────────────────────────

/// Start drawing into `target` (of `size` texels), with the viewport
/// over all of it. A pass already running is suspended until this one
/// ends.
pub fn begin_pass(target: TargetRef, size: (u32, u32), color: Load, depth: Load) {
    PASSES.with(|p| p.borrow_mut().push((target, size)));
    VIEWPORT.with(|v| v.set(Rect::sized(size.0, size.1)));
    device().begin_pass(target, size, color, depth);
}

pub fn begin_window_pass(color: Load, depth: Load) {
    if let Load::Color(c) = color {
        WINDOW_CLEAR.with(|w| w.set(c));
    }
    begin_pass(TargetRef::Window, window_size(), color, depth);
}

/// End the current pass; the pass it interrupted (if any) goes on, with
/// the viewport over all of its target again.
pub fn end_pass() {
    let resume = PASSES.with(|p| {
        let mut passes = p.borrow_mut();
        passes.pop();
        passes.last().copied()
    });
    device().end_pass();
    if let Some((target, size)) = resume {
        VIEWPORT.with(|v| v.set(Rect::sized(size.0, size.1)));
        device().resume_pass(target, size);
    }
}

/// The target being drawn into, if a pass is running.
pub fn current_target() -> Option<TargetRef> {
    PASSES.with(|p| p.borrow().last().map(|(t, _)| *t))
}

/// The size of the target being drawn into.
pub fn current_target_size() -> Option<(u32, u32)> {
    PASSES.with(|p| p.borrow().last().map(|(_, s)| *s))
}

pub fn set_viewport(rect: Rect) {
    VIEWPORT.with(|v| v.set(rect));
    device().set_viewport(rect);
}

/// The viewport set last (by `set_viewport`, or a pass beginning).
pub fn viewport() -> Rect {
    VIEWPORT.with(|v| v.get())
}

pub fn set_scissor(rect: Option<Rect>) {
    device().set_scissor(rect);
}

pub fn set_pipeline(pipeline: Pipeline) {
    device().set_pipeline(pipeline.raw, pipeline.program);
}

pub fn set_stencil_reference(reference: u8) {
    device().set_stencil_reference(reference);
}

pub fn bind_texture(slot: u32, texture: &Texture, sampler: &SamplerDesc) {
    device().bind_texture(slot, texture.raw, texture.desc.kind, self::sampler(sampler));
}

/// Bind a texture known only by its raw id (one made outside `gfx`'s
/// owners, during the move onto it).
pub fn bind_raw_texture(slot: u32, texture: RawId, kind: TextureKind, sampler: &SamplerDesc) {
    device().bind_texture(slot, texture, kind, self::sampler(sampler));
}

pub fn unbind_texture(slot: u32, kind: TextureKind) {
    device().unbind_texture(slot, kind);
}

pub fn bind_uniform_buffer(slot: u32, buffer: &Buffer) {
    device().bind_uniform_buffer(slot, buffer.raw, 0, buffer.size);
}

pub fn bind_uniform_range(slot: u32, buffer: &Buffer, offset: usize, size: usize) {
    device().bind_uniform_buffer(slot, buffer.raw, offset, size);
}

pub fn bind_storage_buffer(slot: u32, buffer: &Buffer) {
    device().bind_storage_buffer(slot, buffer.raw, 0, buffer.size);
}

pub fn bind_storage_range(slot: u32, buffer: &Buffer, offset: usize, size: usize) {
    device().bind_storage_buffer(slot, buffer.raw, offset, size);
}

pub fn set_vertex_buffer(slot: u32, buffer: &Buffer, offset: usize, stride: u32) {
    device().set_vertex_buffer(slot, buffer.raw, offset, stride);
}

/// No vertex buffer in `slot` (attribute-less draws need none at all).
pub fn clear_vertex_buffer(slot: u32) {
    device().set_vertex_buffer(slot, 0, 0, 0);
}

pub fn set_index_buffer(buffer: &Buffer, format: IndexFormat) {
    device().set_index_buffer(buffer.raw, format);
}

/// `set_vertex_buffer` by the buffer's raw id, for a draw recorded earlier
/// in the frame and drawn later (the buffer must still be alive).
pub fn set_raw_vertex_buffer(slot: u32, buffer: RawId, offset: usize, stride: u32) {
    device().set_vertex_buffer(slot, buffer, offset, stride);
}

/// `set_index_buffer` by the buffer's raw id (see `set_raw_vertex_buffer`).
pub fn set_raw_index_buffer(buffer: RawId, format: IndexFormat) {
    device().set_index_buffer(buffer, format);
}

pub fn draw(first_vertex: u32, vertices: u32) {
    if vertices > 0 {
        device().draw(first_vertex, vertices, 0, 1);
    }
}

pub fn draw_instanced(first_vertex: u32, vertices: u32, first_instance: u32, instances: u32) {
    if vertices > 0 && instances > 0 {
        device().draw(first_vertex, vertices, first_instance, instances);
    }
}

pub fn draw_indexed(first_index: u32, indices: u32, base_vertex: i32) {
    if indices > 0 {
        device().draw_indexed(first_index, indices, base_vertex, 0, 1);
    }
}

pub fn draw_indexed_instanced(first_index: u32, indices: u32, base_vertex: i32, first_instance: u32, instances: u32) {
    if indices > 0 && instances > 0 {
        device().draw_indexed(first_index, indices, base_vertex, first_instance, instances);
    }
}

/// Draw into `rect` of the current target only (scissored), cleared to
/// `color` and far depth: an editor preview inside the window.
pub fn begin_region(rect: Rect, color: [f32; 4]) {
    set_scissor(Some(rect));
    set_viewport(rect);
    clear(Planes::COLOR_DEPTH, color, 1.0);
}

/// Leave a region: its depth is cleared (no 3D depth stays behind for
/// what is drawn over it), and the viewport is `full` again.
pub fn end_region(full: Rect) {
    clear(Planes::DEPTH, [0.0; 4], 1.0);
    set_scissor(None);
    set_viewport(full);
}

pub fn clear(planes: Planes, color: [f32; 4], depth: f32) {
    device().clear(planes, color, depth, 0);
}

pub fn blit(src: TargetRef, src_rect: Rect, dst: TargetRef, dst_rect: Rect, planes: Planes, linear: bool) {
    device().blit(src, src_rect, dst, dst_rect, planes, linear);
}

/// RGBA8 rows of `rect` of `target`, the first row (the bottom) first.
pub fn read_pixels(target: TargetRef, rect: Rect) -> Vec<u8> {
    device().read_pixels(target, rect)
}

thread_local! {
    /// `read_pixels_later` of backends that read at once: the next id and
    /// what is not taken yet.
    static EARLY_READS: RefCell<(u64, std::collections::HashMap<u64, Vec<u8>>)> =
        RefCell::new((0, std::collections::HashMap::new()));
}

/// `read_pixels` without waiting for the GPU: ask `take_pixels` with the
/// id in a later frame. Rows as `read_pixels` gives them.
pub fn read_pixels_later(target: TargetRef, rect: Rect) -> u64 {
    device().read_pixels_later(target, rect)
}

/// The pixels of a `read_pixels_later`, once they are there.
pub fn take_pixels(id: u64) -> Option<Vec<u8>> {
    device().take_pixels(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_device_everything_is_inert() {
        assert_eq!(backend(), BackendKind::Null);
        let mut buffer = Buffer::new(BufferUsage::VERTEX, BufferMemory::Static, 16);
        buffer.write(0, &[1, 2, 3, 4]);
        buffer.replace(&[0; 64]);
        assert_eq!(buffer.size(), 64);
        let texture = Texture::new(TextureDesc::d2(Format::Rgba8, 4, 4));
        texture.write(0, Rect::sized(4, 4), 0, 1, UploadFormat::Rgba8, &[0; 64]);
        begin_window_pass(Load::Color([0.0; 4]), Load::Depth(1.0));
        assert_eq!(current_target(), Some(TargetRef::Window));
        draw(0, 3);
        end_pass();
        assert_eq!(current_target(), None);
        collect_garbage();
    }

    #[test]
    fn nested_passes_resume_the_outer_one() {
        begin_pass(TargetRef::Target(7), (8, 8), Load::Keep, Load::Keep);
        begin_pass(TargetRef::Target(9), (4, 4), Load::Keep, Load::Keep);
        assert_eq!(current_target_size(), Some((4, 4)));
        end_pass();
        assert_eq!(current_target(), Some(TargetRef::Target(7)));
        end_pass();
        assert_eq!(current_target(), None);
    }
}
