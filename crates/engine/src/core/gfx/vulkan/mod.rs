//! The Vulkan 1.3 backend (dynamic rendering, synchronization2, push
//! descriptors), drawing the same image as the GL backend with GL's
//! conventions kept (see docs/render-backends.md, decision 3):
//!
//! - depth in [-1, 1] (`VK_EXT_depth_clip_control`), a positive-height
//!   viewport (row 0 is NDC y = −1, as in GL) and clockwise front faces
//!   (GL's counter-clockwise ones seen from the other side of the flip);
//! - the window is an offscreen target (multisampled colour and
//!   depth-stencil, like GL's default framebuffer), resolved and blitted
//!   upside down into the swapchain image when the frame ends.
//!
//! Commands are recorded as they come, into one command buffer per frame
//! (two frames in flight). Rendering into a pass's target begins lazily
//! at its first draw or clear; whatever cannot happen while rendering (a
//! copy, a blit, a layout change of a sampled texture) ends it, and the
//! next draw begins it again, keeping the contents. Buffer uploads go
//! into a second command buffer that runs before the frame's main one.
//!
//! Synchronisation is deliberately coarse for now: a global memory
//! barrier (all commands to all commands) wherever rendering begins and
//! before every transfer, together with the image layout changes. Finer
//! barriers are a later optimisation.

mod context;
mod convert;
mod pipeline;
mod record;
mod resources;
mod spirv;

use std::{
    cell::RefCell,
    mem::ManuallyDrop,
};

use ash::vk;
use gpu_allocator::vulkan::{Allocation, Allocator};

use super::*;
use super::fx::{FxHashMap, FxHashSet};
use context::{Gpu, Swapchain};

/// Frames recorded or executing at once.
const FRAMES: usize = 2;

pub(super) struct VkDevice {
    state: RefCell<State>,
}

/// Something to destroy once the GPU is done with it.
enum Retired {
    /// Buffer, its memory, its size (for reuse).
    Buffer(vk::Buffer, Allocation, usize),
    Image(vk::Image, Allocation, Vec<vk::ImageView>),
    Pipeline(vk::Pipeline),
    Program(vk::ShaderModule, vk::ShaderModule, vk::DescriptorSetLayout, vk::PipelineLayout),
    /// The params blocks' set layout of a program.
    SetLayout(vk::DescriptorSetLayout),
    /// Params descriptor sets (from `params_pool`).
    Sets(Vec<vk::DescriptorSet>),
    Queries(vk::QueryPool),
}

/// A piece of per-frame host-visible memory: staging data and uniform
/// parameters, handed out front to back.
struct Chunk {
    buffer: vk::Buffer,
    allocation: Allocation,
    ptr: *mut u8,
    size: u64,
    used: u64,
}

struct Frame {
    pool: vk::CommandPool,
    /// Buffer uploads, run before `main`.
    upload: vk::CommandBuffer,
    main: vk::CommandBuffer,
    fence: vk::Fence,
    /// Signalled when the swapchain image is acquired.
    acquired: vk::Semaphore,
    /// Serial of the commands last submitted from here.
    serial: u64,
    arena: Vec<Chunk>,
    /// Uniform params uploaded this frame (see `params_alloc`).
    params: Option<Chunk>,
}

struct Buf {
    buffer: vk::Buffer,
    allocation: Option<Allocation>,
    size: usize,
    memory: BufferMemory,
    /// Mapped memory of a dynamic buffer.
    ptr: *mut u8,
    /// End of what was written since this storage was made.
    high_water: usize,
    /// Serial of the last commands that used it.
    used: u64,
}

struct Tex {
    image: vk::Image,
    allocation: Option<Allocation>,
    desc: TextureDesc,
    format: vk::Format,
    mips: u32,
    layers: u32,
    /// Layout of each subresource, mip-major (`mip * layers + layer`).
    layouts: Vec<vk::ImageLayout>,
    /// The view shaders sample (null for multisampled textures).
    view: vk::ImageView,
    /// Views for render target attachments, by mip and layer.
    attachment_views: FxHashMap<(u32, Option<u32>), vk::ImageView>,
}

struct Prog {
    label: String,
    reflect: glsl::VulkanProgram,
    vertex: vk::ShaderModule,
    fragment: vk::ShaderModule,
    set_layout: vk::DescriptorSetLayout,
    layout: vk::PipelineLayout,
    /// The params blocks as the shaders see them (std140).
    blocks: Vec<ParamBlock>,
    /// Layout of the params blocks' set (`glsl::PARAMS_SET`); null
    /// without loose uniforms.
    params_layout: vk::DescriptorSetLayout,
    /// The texture unit each sampler reads (set by `set_uniform`).
    units: Vec<u32>,
    /// View kind and whether it compares, per sampler.
    sampler_kinds: Vec<(TextureKind, bool)>,
    /// Buffer blocks, one per slot.
    buffers: Vec<glsl::BufferBlock>,
    /// The vertex stage's inputs: location and scalar type.
    inputs: Vec<(u32, spirv::Scalar)>,
    /// Uniform names: a sampler's index (`Ok`) or a param's (`Err`).
    names: FxHashMap<String, Result<usize, usize>>,
}

struct ParamBlock {
    data: Vec<u8>,
    /// Where it was uploaded last (serial, offset in that frame's params
    /// memory); `None` once it changed.
    upload: Option<(u64, u32)>,
}

/// At most this many params blocks per program (dynamic uniform buffers
/// a pipeline may have: 8 at least).
pub(super) const MAX_PARAM_BLOCKS: usize = 8;

struct PipeEntry {
    program: RawId,
    desc: PipelineDesc,
    vao: usize,
    /// Whether some vertex input of the program no layout feeds (it
    /// reads the default attribute).
    missing: bool,
}

/// Colour attachment formats of a rendering (at most `MAX_COLORS`).
const MAX_COLORS: usize = 4;
type ColorFormats = ([vk::Format; MAX_COLORS], u8);

/// A sampler binding of a draw: binding, view, sampler, texture.
type ImageBinding = (u32, vk::ImageView, vk::Sampler, RawId);

/// GL's vertex array object: what is bound for one set of vertex layouts.
#[derive(Default, Clone)]
struct Vao {
    vertex: [Option<(RawId, usize, u32)>; 16],
    index: Option<RawId>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PipelineKey {
    program: RawId,
    desc: PipelineDesc,
    line: bool,
    colors: ColorFormats,
    depth: vk::Format,
    stencil: bool,
    samples: u32,
}

struct QuerySet {
    pool: vk::QueryPool,
    /// Serial of the commands that wrote each query (0: never written).
    written: Vec<u64>,
}

/// The pass being drawn.
#[derive(Clone, Copy)]
struct Pass {
    target: TargetRef,
    color: Load,
    depth: Load,
}

/// The window's offscreen target.
struct WindowTarget {
    color: RawId,
    depth: RawId,
    resolve: RawId,
    size: (u32, u32),
}

struct State {
    gpu: Gpu,
    allocator: ManuallyDrop<Allocator>,
    swapchain: Option<Swapchain>,
    vsync: bool,
    swapchain_dirty: bool,
    window: Option<WindowTarget>,
    frames: Vec<Frame>,
    frame: usize,
    /// Serial of the commands being recorded; the next submission gets it.
    serial: u64,
    /// Every submission up to this serial has finished.
    completed: u64,
    retired: Vec<(u64, Retired)>,
    /// Dynamic buffer storage freed, kept for reuse: by size.
    spare: FxHashMap<usize, Vec<(vk::Buffer, Allocation)>>,
    next_id: RawId,
    buffers: FxHashMap<RawId, Buf>,
    textures: FxHashMap<RawId, Tex>,
    /// Each sampler: its description and its Vulkan sampler without and
    /// with depth comparison (made when first needed).
    samplers: FxHashMap<RawId, (SamplerDesc, [vk::Sampler; 2])>,
    sampler_variants: Vec<(SamplerDesc, vk::Sampler)>,
    /// The pipeline the last draw used, by key: consecutive draws mostly
    /// share it, which spares hashing the key.
    last_vk_pipeline: Option<(PipelineKey, vk::Pipeline)>,
    targets: FxHashMap<RawId, (Vec<Attachment>, Option<Attachment>)>,
    programs: FxHashMap<RawId, Prog>,
    pipelines: FxHashMap<RawId, PipeEntry>,
    vk_pipelines: FxHashMap<PipelineKey, vk::Pipeline>,
    pipeline_cache: vk::PipelineCache,
    vaos: Vec<Vao>,
    vao_index: FxHashMap<Layouts, usize>,
    queries: FxHashMap<RawId, QuerySet>,
    /// Buffers written by the upload commands since their last barrier.
    upload_dirty: FxHashSet<vk::Buffer>,
    compiler: Option<shaderc::Compiler>,

    // Recording.
    pass: Option<Pass>,
    rendering: bool,
    /// Formats and samples of the rendering going on.
    rendering_key: (ColorFormats, vk::Format, bool, u32),
    render_extent: (u32, u32),
    render_layers: u32,
    viewport: Rect,
    scissor: Option<Rect>,
    dynamic_dirty: bool,
    pipeline: Option<RawId>,
    resolved: Option<PipelineDesc>,
    line: bool,
    stencil_reference: u8,
    bound_pipeline: vk::Pipeline,
    units: FxHashMap<(u32, TextureKind), (RawId, RawId)>,
    uniform_buffers: FxHashMap<u32, (RawId, usize, usize)>,
    storage_buffers: FxHashMap<u32, (RawId, usize, usize)>,
    index_type: vk::IndexType,

    // Stand-ins for what a draw reads but nothing was bound for.
    dummy_textures: FxHashMap<(TextureKind, bool), RawId>,
    dummy_buffer: RawId,
    /// Zeros with w = 1, for vertex inputs no buffer feeds.
    default_attribute: RawId,
    warned: FxHashSet<String>,

    // Scratch reused by every draw (no allocation per draw).
    scratch_images: Vec<ImageBinding>,
    scratch_sampled: Vec<RawId>,
    scratch_push: Vec<u64>,
    /// What was pushed last (layout and descriptors, see
    /// `push_descriptors`): an identical push is left out.
    last_push: Vec<u64>,

    /// Pool of the params sets (`Prog::params_layout`), made at start.
    params_pool: vk::DescriptorPool,
    /// A params set per program and params memory it reads.
    params_sets: FxHashMap<(RawId, vk::Buffer), vk::DescriptorSet>,
    /// The params set and offsets bound last (none after a new command
    /// buffer).
    last_params: Option<(vk::PipelineLayout, vk::DescriptorSet, [u32; MAX_PARAM_BLOCKS])>,
}

impl State {
    fn id(&mut self) -> RawId {
        self.next_id += 1;
        self.next_id
    }

    fn frame(&self) -> &Frame {
        &self.frames[self.frame]
    }

    fn cmd(&self) -> vk::CommandBuffer {
        self.frames[self.frame].main
    }

    fn upload_cmd(&self) -> vk::CommandBuffer {
        self.frames[self.frame].upload
    }

    fn warn_once(&mut self, message: String) {
        if self.warned.insert(message.clone()) {
            log::warn!("Vulkan: {message}");
        }
    }

    fn retire(&mut self, r: Retired) {
        let serial = self.serial;
        self.retired.push((serial, r));
    }

    /// Free what the GPU is done with.
    fn free_retired(&mut self) {
        let completed = self.completed;
        let retired = std::mem::take(&mut self.retired);
        for (serial, r) in retired {
            if serial > completed {
                self.retired.push((serial, r));
                continue;
            }
            unsafe {
                match r {
                    Retired::Buffer(b, a, size) => {
                        let spare = self.spare.entry(size).or_default();
                        if spare.len() < 8 && a.mapped_ptr().is_some() {
                            spare.push((b, a));
                        } else {
                            self.gpu.device.destroy_buffer(b, None);
                            let _ = self.allocator.free(a);
                        }
                    }
                    Retired::Image(i, a, views) => {
                        for v in views {
                            self.gpu.device.destroy_image_view(v, None);
                        }
                        self.gpu.device.destroy_image(i, None);
                        let _ = self.allocator.free(a);
                    }
                    Retired::Pipeline(p) => self.gpu.device.destroy_pipeline(p, None),
                    Retired::Program(v, f, s, l) => {
                        self.gpu.device.destroy_shader_module(v, None);
                        self.gpu.device.destroy_shader_module(f, None);
                        self.gpu.device.destroy_descriptor_set_layout(s, None);
                        self.gpu.device.destroy_pipeline_layout(l, None);
                    }
                    Retired::Queries(q) => self.gpu.device.destroy_query_pool(q, None),
                    Retired::SetLayout(l) => self.gpu.device.destroy_descriptor_set_layout(l, None),
                    Retired::Sets(sets) => {
                        let _ = self.gpu.device.free_descriptor_sets(self.params_pool, &sets);
                    }
                }
            }
        }
    }

    /// Note which submissions have finished, without waiting.
    fn poll_completed(&mut self) {
        for f in &self.frames {
            if f.serial > self.completed && unsafe { self.gpu.device.get_fence_status(f.fence) } == Ok(true) {
                self.completed = self.completed.max(f.serial);
            }
        }
    }

    /// Start recording into the current frame's command buffers.
    fn begin_commands(&mut self) {
        let frame = &self.frames[self.frame];
        unsafe {
            let device = &self.gpu.device;
            let _ = device.reset_command_pool(frame.pool, vk::CommandPoolResetFlags::empty());
            let begin = vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
            let _ = device.begin_command_buffer(frame.upload, &begin);
            let _ = device.begin_command_buffer(frame.main, &begin);
        }
        // Whatever ran before is done before this frame's uploads write.
        self.global_barrier(self.upload_cmd());
        self.upload_dirty.clear();
        self.bound_pipeline = vk::Pipeline::null();
        self.last_push.clear();
        self.last_params = None;
        self.dynamic_dirty = true;
        self.rendering = false;
    }

    /// End and submit the current frame's command buffers, waiting on
    /// `wait` and signalling `signal` (when not null).
    fn submit(&mut self, wait: vk::Semaphore, signal: vk::Semaphore) {
        self.interrupt_rendering();
        let upload = self.upload_cmd();
        self.global_barrier(upload);
        let frame = &mut self.frames[self.frame];
        frame.serial = self.serial;
        unsafe {
            let device = &self.gpu.device;
            let _ = device.end_command_buffer(frame.upload);
            let _ = device.end_command_buffer(frame.main);
            let _ = device.reset_fences(&[frame.fence]);
            let buffers = [
                vk::CommandBufferSubmitInfo::default().command_buffer(frame.upload),
                vk::CommandBufferSubmitInfo::default().command_buffer(frame.main),
            ];
            let waits = [vk::SemaphoreSubmitInfo::default()
                .semaphore(wait)
                .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)];
            let signals = [vk::SemaphoreSubmitInfo::default()
                .semaphore(signal)
                .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)];
            let mut submit = vk::SubmitInfo2::default().command_buffer_infos(&buffers);
            if wait != vk::Semaphore::null() {
                submit = submit.wait_semaphore_infos(&waits);
            }
            if signal != vk::Semaphore::null() {
                submit = submit.signal_semaphore_infos(&signals);
            }
            if let Err(e) = device.queue_submit2(self.gpu.queue, &[submit], frame.fence) {
                log::error!("Vulkan submit failed: {e}");
            }
        }
        self.serial += 1;
    }

    /// Make the current frame slot ready for recording again: wait for its
    /// last submission, free what it held.
    fn recycle_frame(&mut self) {
        let frame = &mut self.frames[self.frame];
        unsafe {
            let _ = self.gpu.device.wait_for_fences(&[frame.fence], true, u64::MAX);
        }
        self.completed = self.completed.max(frame.serial);
        for chunk in &mut frame.arena {
            chunk.used = 0;
        }
        if let Some(params) = frame.params.as_mut() {
            params.used = 0;
        }
        // Keep the first chunks (the usual size); drop extra large ones.
        let mut keep = Vec::new();
        for chunk in std::mem::take(&mut frame.arena) {
            if keep.len() < 2 && chunk.size == resources::CHUNK_SIZE {
                keep.push(chunk);
            } else {
                unsafe { self.gpu.device.destroy_buffer(chunk.buffer, None) };
                let _ = self.allocator.free(chunk.allocation);
            }
        }
        frame.arena = keep;
        self.poll_completed();
        self.free_retired();
        self.begin_commands();
    }

    /// Submit what was recorded and wait for the GPU to finish it, then go
    /// on recording (a pass going on resumes with its contents).
    fn flush_and_wait(&mut self) {
        self.submit(vk::Semaphore::null(), vk::Semaphore::null());
        self.recycle_frame();
    }

    /// Bytes of the current frame's staging and parameter memory in use.
    fn arena_used(&self) -> u64 {
        self.frame().arena.iter().map(|c| c.used).sum()
    }

    fn global_barrier(&self, cmd: vk::CommandBuffer) {
        let barrier = [vk::MemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
            .src_access_mask(vk::AccessFlags2::MEMORY_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
            .dst_access_mask(vk::AccessFlags2::MEMORY_READ | vk::AccessFlags2::MEMORY_WRITE)];
        unsafe {
            self.gpu
                .device
                .cmd_pipeline_barrier2(cmd, &vk::DependencyInfo::default().memory_barriers(&barrier));
        }
    }
}

/// The Vulkan device for a window: `extensions` are the instance
/// extensions the window system needs, `create_surface` makes the
/// window's surface (raw `VkInstance` in, raw `VkSurfaceKHR` out).
pub(super) fn create(
    extensions: &[String],
    create_surface: &dyn Fn(u64) -> Result<u64, String>,
    vsync: bool,
) -> Result<Box<dyn Device>, String> {
    use ash::vk::Handle;
    let gpu = context::create(extensions, &|instance| {
        create_surface(instance.as_raw()).map(vk::SurfaceKHR::from_raw)
    })?;
    let allocator = Allocator::new(&gpu_allocator::vulkan::AllocatorCreateDesc {
        instance: gpu.instance.clone(),
        device: gpu.device.clone(),
        physical_device: gpu.physical,
        debug_settings: Default::default(),
        buffer_device_address: false,
        allocation_sizes: Default::default(),
    })
    .map_err(|e| format!("allocator: {e}"))?;
    let mut frames = Vec::new();
    unsafe {
        let device = &gpu.device;
        for _ in 0..FRAMES {
            let pool = device
                .create_command_pool(&vk::CommandPoolCreateInfo::default().queue_family_index(gpu.family), None)
                .map_err(|e| format!("command pool: {e}"))?;
            let buffers = device
                .allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(pool)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count(2),
                )
                .map_err(|e| format!("command buffers: {e}"))?;
            let fence = device
                .create_fence(&vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED), None)
                .map_err(|e| format!("fence: {e}"))?;
            let acquired = device
                .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
                .map_err(|e| format!("semaphore: {e}"))?;
            frames.push(Frame {
                pool,
                upload: buffers[0],
                main: buffers[1],
                fence,
                acquired,
                serial: 0,
                arena: Vec::new(),
                params: None,
            });
        }
    }
    // Pipelines made in earlier runs: the driver checks the data is its
    // own (device, driver version) and starts empty otherwise.
    let cached = std::fs::read(pipeline_cache_path()).unwrap_or_default();
    let pipeline_cache = unsafe {
        gpu.device
            .create_pipeline_cache(&vk::PipelineCacheCreateInfo::default().initial_data(&cached), None)
            .or_else(|_| gpu.device.create_pipeline_cache(&vk::PipelineCacheCreateInfo::default(), None))
    }
    .unwrap_or_default();
    let swapchain = context::create_swapchain(&gpu, vk::SwapchainKHR::null(), super::window_size(), vsync)?;
    let compiler = shaderc::Compiler::new().map_err(|e| format!("shaderc: {e}"))?;
    // The params sets: one per program and params memory, freed with the
    // program.
    let params_pool = unsafe {
        let sizes = [vk::DescriptorPoolSize {
            ty: vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC,
            descriptor_count: 4096 * MAX_PARAM_BLOCKS as u32,
        }];
        gpu.device
            .create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .flags(vk::DescriptorPoolCreateFlags::FREE_DESCRIPTOR_SET)
                    .max_sets(4096)
                    .pool_sizes(&sizes),
                None,
            )
            .map_err(|e| format!("descriptor pool: {e}"))?
    };
    let mut state = State {
        gpu,
        allocator: ManuallyDrop::new(allocator),
        swapchain: Some(swapchain),
        vsync,
        swapchain_dirty: false,
        window: None,
        frames,
        frame: 0,
        serial: 1,
        completed: 0,
        retired: Vec::new(),
        spare: FxHashMap::default(),
        next_id: 0,
        buffers: FxHashMap::default(),
        textures: FxHashMap::default(),
        samplers: FxHashMap::default(),
        sampler_variants: Vec::new(),
        last_vk_pipeline: None,
        targets: FxHashMap::default(),
        programs: FxHashMap::default(),
        pipelines: FxHashMap::default(),
        vk_pipelines: FxHashMap::default(),
        pipeline_cache,
        vaos: vec![Vao::default()],
        vao_index: FxHashMap::default(),
        queries: FxHashMap::default(),
        upload_dirty: FxHashSet::default(),
        compiler: Some(compiler),
        pass: None,
        rendering: false,
        rendering_key: (([vk::Format::UNDEFINED; MAX_COLORS], 0), vk::Format::UNDEFINED, false, 1),
        render_extent: (1, 1),
        render_layers: 1,
        viewport: Rect::sized(1, 1),
        scissor: None,
        dynamic_dirty: true,
        pipeline: None,
        resolved: None,
        line: false,
        stencil_reference: 0,
        bound_pipeline: vk::Pipeline::null(),
        units: FxHashMap::default(),
        uniform_buffers: FxHashMap::default(),
        storage_buffers: FxHashMap::default(),
        index_type: vk::IndexType::UINT32,
        dummy_textures: FxHashMap::default(),
        dummy_buffer: 0,
        default_attribute: 0,
        warned: FxHashSet::default(),
        scratch_images: Vec::new(),
        scratch_sampled: Vec::new(),
        scratch_push: Vec::new(),
        last_push: Vec::new(),
        params_sets: FxHashMap::default(),
        last_params: None,
        params_pool,
    };
    state.begin_commands();
    state.create_dummies();
    state.update_window_target();
    Ok(Box::new(VkDevice { state: RefCell::new(state) }))
}

impl VkDevice {
    fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        f(&mut self.state.borrow_mut())
    }
}

impl Device for VkDevice {
    fn kind(&self) -> BackendKind {
        BackendKind::Vulkan
    }

    fn describe(&self) -> String {
        self.with(|s| {
            let v = s.gpu.props.api_version;
            format!(
                "{}, Vulkan {}.{}.{} ({})",
                s.gpu.name(),
                vk::api_version_major(v),
                vk::api_version_minor(v),
                vk::api_version_patch(v),
                s.gpu.driver.trim()
            )
        })
    }

    fn video_memory(&self) -> Option<(u64, u64)> {
        self.with(|s| s.gpu.memory_budget())
    }

    fn create_buffer(&self, usage: BufferUsage, memory: BufferMemory, size: usize, data: Option<&[u8]>) -> RawId {
        self.with(|s| s.create_buffer(usage, memory, size, data))
    }

    fn write_buffer(&self, buffer: RawId, offset: usize, data: &[u8], orphan: bool) {
        self.with(|s| s.write_buffer(buffer, offset, data, orphan))
    }

    fn resize_buffer(&self, buffer: RawId, size: usize) {
        self.with(|s| s.resize_buffer(buffer, size))
    }

    fn create_texture(&self, desc: &TextureDesc) -> RawId {
        self.with(|s| s.create_texture(desc))
    }

    fn write_texture(
        &self,
        texture: RawId,
        _desc: &TextureDesc,
        mip: u32,
        region: Rect,
        layer: u32,
        layers: u32,
        format: UploadFormat,
        data: &[u8],
    ) {
        self.with(|s| s.write_texture(texture, mip, region, layer, layers, format, data))
    }

    fn generate_mips(&self, texture: RawId, _desc: &TextureDesc) {
        self.with(|s| s.generate_mips(texture))
    }

    fn create_sampler(&self, desc: &SamplerDesc) -> RawId {
        self.with(|s| {
            let id = s.id();
            s.samplers.insert(id, (*desc, [vk::Sampler::null(); 2]));
            id
        })
    }

    fn create_target(&self, colors: &[Attachment], depth: Option<Attachment>) -> RawId {
        self.with(|s| {
            let id = s.id();
            s.targets.insert(id, (colors.to_vec(), depth));
            id
        })
    }

    fn create_program(&self, vertex: &str, fragment: &str, label: &str) -> Result<RawId, String> {
        self.with(|s| s.create_program(vertex, fragment, label))
    }

    fn create_pipeline(&self, program: RawId, desc: &PipelineDesc) -> RawId {
        self.with(|s| {
            let vao = match s.vao_index.get(&desc.vertex_layouts) {
                Some(&v) => v,
                None => {
                    s.vaos.push(Vao::default());
                    let v = s.vaos.len() - 1;
                    s.vao_index.insert(desc.vertex_layouts.clone(), v);
                    v
                }
            };
            let id = s.id();
            let missing = !s.missing_inputs(program, &desc.vertex_layouts).is_empty();
            s.pipelines.insert(id, PipeEntry { program, desc: *desc, vao, missing });
            id
        })
    }

    fn set_uniform(&self, program: RawId, name: &str, value: UniformValue) {
        self.with(|s| s.set_uniform(program, name, value))
    }

    fn create_queries(&self, count: u32) -> RawId {
        self.with(|s| s.create_queries(count))
    }

    fn write_timestamp(&self, queries: RawId, index: u32) {
        self.with(|s| s.write_timestamp(queries, index))
    }

    fn read_timestamps(&self, queries: RawId, count: u32) -> Option<Vec<u64>> {
        self.with(|s| s.read_timestamps(queries, count))
    }

    fn destroy(&self, raw: Raw) {
        self.with(|s| s.destroy(raw))
    }

    fn begin_pass(&self, target: TargetRef, size: (u32, u32), color: Load, depth: Load) {
        self.with(|s| s.begin_pass(target, size, color, depth))
    }

    fn resume_pass(&self, target: TargetRef, size: (u32, u32)) {
        self.with(|s| s.begin_pass(target, size, Load::Keep, Load::Keep))
    }

    fn end_pass(&self) {
        self.with(|s| s.end_pass())
    }

    fn set_viewport(&self, rect: Rect) {
        self.with(|s| {
            s.viewport = rect;
            s.dynamic_dirty = true;
        })
    }

    fn set_scissor(&self, rect: Option<Rect>) {
        self.with(|s| {
            s.scissor = rect;
            s.dynamic_dirty = true;
        })
    }

    fn set_pipeline(&self, pipeline: RawId, _program: RawId) {
        self.with(|s| s.set_pipeline(pipeline))
    }

    fn set_stencil_reference(&self, reference: u8) {
        self.with(|s| {
            s.stencil_reference = reference;
            s.dynamic_dirty = true;
        })
    }

    fn bind_texture(&self, slot: u32, texture: RawId, kind: TextureKind, sampler: RawId) {
        self.with(|s| {
            s.units.insert((slot, kind), (texture, sampler));
        })
    }

    fn unbind_texture(&self, slot: u32, kind: TextureKind) {
        self.with(|s| {
            s.units.remove(&(slot, kind));
        })
    }

    fn bind_uniform_buffer(&self, slot: u32, buffer: RawId, offset: usize, size: usize) {
        self.with(|s| {
            s.uniform_buffers.insert(slot, (buffer, offset, size));
        })
    }

    fn bind_storage_buffer(&self, slot: u32, buffer: RawId, offset: usize, size: usize) {
        self.with(|s| {
            s.storage_buffers.insert(slot, (buffer, offset, size));
        })
    }

    fn set_vertex_buffer(&self, slot: u32, buffer: RawId, offset: usize, stride: u32) {
        self.with(|s| {
            let vao = s.pipeline.and_then(|p| s.pipelines.get(&p)).map_or(0, |p| p.vao);
            if let Some(v) = s.vaos[vao].vertex.get_mut(slot as usize) {
                *v = (buffer != 0).then_some((buffer, offset, stride));
            }
        })
    }

    fn set_index_buffer(&self, buffer: RawId, format: IndexFormat) {
        self.with(|s| {
            s.index_type = match format {
                IndexFormat::U16 => vk::IndexType::UINT16,
                IndexFormat::U32 => vk::IndexType::UINT32,
            };
            let vao = s.pipeline.and_then(|p| s.pipelines.get(&p)).map_or(0, |p| p.vao);
            s.vaos[vao].index = (buffer != 0).then_some(buffer);
        })
    }

    fn draw(&self, first_vertex: u32, vertices: u32, first_instance: u32, instances: u32) {
        self.with(|s| s.draw(None, first_vertex, vertices, first_instance, instances))
    }

    fn draw_indexed(&self, first_index: u32, indices: u32, base_vertex: i32, first_instance: u32, instances: u32) {
        self.with(|s| s.draw(Some(base_vertex), first_index, indices, first_instance, instances))
    }

    fn clear(&self, planes: Planes, color: [f32; 4], depth: f32, stencil: u8) {
        self.with(|s| s.clear(planes, color, depth, stencil))
    }

    fn blit(&self, src: TargetRef, src_rect: Rect, dst: TargetRef, dst_rect: Rect, planes: Planes, linear: bool) {
        self.with(|s| s.blit(src, src_rect, dst, dst_rect, planes, linear))
    }

    fn read_pixels(&self, target: TargetRef, rect: Rect) -> Vec<u8> {
        self.with(|s| s.read_pixels(target, rect))
    }

    fn window_samples(&self) -> u32 {
        self.with(|s| s.gpu.window_samples)
    }

    fn begin_frame(&self) {
        self.with(|s| s.update_window_target())
    }

    fn end_frame(&self) {
        self.with(|s| s.end_frame())
    }

    fn set_vsync(&self, on: bool) {
        self.with(|s| {
            if s.vsync != on {
                s.vsync = on;
                s.swapchain_dirty = true;
            }
        })
    }

    fn shutdown(&self) {
        self.with(|s| unsafe {
            let _ = s.gpu.device.device_wait_idle();
            save_pipeline_cache(&s.gpu.device, s.pipeline_cache);
            if let Some(swapchain) = s.swapchain.take() {
                context::destroy_swapchain(&s.gpu, &swapchain);
            }
            s.gpu.surface_fn.destroy_surface(s.gpu.surface, None);
            if let Some((debug_fn, messenger)) = s.gpu.debug.take() {
                debug_fn.destroy_debug_utils_messenger(messenger, None);
            }
        })
    }
}

/// Where the pipeline cache is kept between runs (the user's cache
/// directory).
pub(super) fn cache_dir() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("ferrite").join("vulkan")
}

fn pipeline_cache_path() -> std::path::PathBuf {
    cache_dir().join("pipelines.bin")
}

/// Keep the pipelines made this run for the next (written atomically).
fn save_pipeline_cache(device: &ash::Device, cache: vk::PipelineCache) {
    if cache == vk::PipelineCache::null() {
        return;
    }
    let Ok(data) = (unsafe { device.get_pipeline_cache_data(cache) }) else {
        return;
    };
    let path = pipeline_cache_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let temp = path.with_extension("tmp");
    if std::fs::write(&temp, &data).is_ok() {
        let _ = std::fs::rename(&temp, &path);
    }
}
