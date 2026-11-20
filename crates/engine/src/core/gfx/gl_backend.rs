//! The OpenGL 4.5 backend (direct state access).
//!
//! Raw ids are GL object names, except pipelines (an index into the
//! pipeline list) and query sets (an index into the query list). Sampler
//! state is kept on the texture objects, set when a texture is bound
//! with a different sampler than last time: code not yet moved onto the
//! render layer binds textures itself and must not meet a sampler object
//! left on a unit.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    ffi::CString,
    ptr,
};

use gl::types::*;

use super::*;

pub(super) struct GlDevice {
    pipelines: RefCell<Vec<GlPipeline>>,
    /// One vertex array per distinct set of vertex layouts.
    vertex_arrays: RefCell<HashMap<Vec<VertexLayout>, GLuint>>,
    /// The vertex array of the current pipeline.
    current_vao: Cell<GLuint>,
    current_topology: Cell<GLenum>,
    index_format: Cell<GLenum>,
    /// The sampler last applied to each texture.
    texture_samplers: RefCell<HashMap<GLuint, RawId>>,
    samplers: RefCell<Vec<SamplerDesc>>,
    queries: RefCell<Vec<Vec<GLuint>>>,
    /// Attachments of each target, by framebuffer name.
    targets: RefCell<HashMap<GLuint, TargetInfo>>,
    wireframe: Cell<bool>,
    stencil: Cell<Option<StencilState>>,
    stencil_reference: Cell<u8>,
}

struct GlPipeline {
    program: GLuint,
    desc: PipelineDesc,
    vao: GLuint,
}

struct TargetInfo {
    colors: usize,
}

impl GlDevice {
    pub(super) fn new() -> Self {
        GlDevice {
            pipelines: RefCell::new(Vec::new()),
            vertex_arrays: RefCell::new(HashMap::new()),
            current_vao: Cell::new(0),
            current_topology: Cell::new(gl::TRIANGLES),
            index_format: Cell::new(gl::UNSIGNED_INT),
            texture_samplers: RefCell::new(HashMap::new()),
            samplers: RefCell::new(Vec::new()),
            queries: RefCell::new(Vec::new()),
            targets: RefCell::new(HashMap::new()),
            wireframe: Cell::new(false),
            stencil: Cell::new(None),
            stencil_reference: Cell::new(0),
        }
    }

    fn vertex_array(&self, layouts: &[VertexLayout]) -> GLuint {
        if let Some(&vao) = self.vertex_arrays.borrow().get(layouts) {
            return vao;
        }
        let mut vao = 0;
        unsafe {
            gl::CreateVertexArrays(1, &mut vao);
            for (slot, layout) in layouts.iter().enumerate() {
                let slot = slot as GLuint;
                for attribute in &layout.attributes {
                    gl::EnableVertexArrayAttrib(vao, attribute.location);
                    let components = attribute.format.components() as GLint;
                    if attribute.format.is_integer() {
                        gl::VertexArrayAttribIFormat(vao, attribute.location, components, gl::UNSIGNED_INT, attribute.offset);
                    } else {
                        gl::VertexArrayAttribFormat(vao, attribute.location, components, gl::FLOAT, gl::FALSE, attribute.offset);
                    }
                    gl::VertexArrayAttribBinding(vao, attribute.location, slot);
                }
                let divisor = match layout.step {
                    StepMode::Vertex => 0,
                    StepMode::Instance => 1,
                };
                gl::VertexArrayBindingDivisor(vao, slot, divisor);
            }
        }
        self.vertex_arrays.borrow_mut().insert(layouts.to_vec(), vao);
        vao
    }

    fn bind_framebuffer(&self, target: TargetRef) {
        unsafe { crate::core::renderer::framebuffer::bind(gl::FRAMEBUFFER, framebuffer_of(target)) };
    }

    fn apply_sampler(&self, texture: GLuint, sampler: RawId) {
        if sampler == 0 || self.texture_samplers.borrow().get(&texture) == Some(&sampler) {
            return;
        }
        let Some(desc) = self.samplers.borrow().get(sampler as usize - 1).copied() else {
            return;
        };
        self.texture_samplers.borrow_mut().insert(texture, sampler);
        unsafe {
            let min = match (desc.min, desc.mip) {
                (Filter::Nearest, None) => gl::NEAREST,
                (Filter::Linear, None) => gl::LINEAR,
                (Filter::Nearest, Some(Filter::Nearest)) => gl::NEAREST_MIPMAP_NEAREST,
                (Filter::Nearest, Some(Filter::Linear)) => gl::NEAREST_MIPMAP_LINEAR,
                (Filter::Linear, Some(Filter::Nearest)) => gl::LINEAR_MIPMAP_NEAREST,
                (Filter::Linear, Some(Filter::Linear)) => gl::LINEAR_MIPMAP_LINEAR,
            };
            gl::TextureParameteri(texture, gl::TEXTURE_MIN_FILTER, min as GLint);
            gl::TextureParameteri(texture, gl::TEXTURE_MAG_FILTER, filter(desc.mag) as GLint);
            gl::TextureParameteri(texture, gl::TEXTURE_WRAP_S, wrap(desc.wrap[0]) as GLint);
            gl::TextureParameteri(texture, gl::TEXTURE_WRAP_T, wrap(desc.wrap[1]) as GLint);
            gl::TextureParameteri(texture, gl::TEXTURE_WRAP_R, wrap(desc.wrap[2]) as GLint);
            let border = match desc.border {
                BorderColor::TransparentBlack => [0.0f32; 4],
                BorderColor::OpaqueBlack => [0.0, 0.0, 0.0, 1.0],
                BorderColor::OpaqueWhite => [1.0; 4],
            };
            gl::TextureParameterfv(texture, gl::TEXTURE_BORDER_COLOR, border.as_ptr());
            // GL_TEXTURE_MAX_ANISOTROPY (core in 4.6, ARB/EXT before).
            gl::TextureParameterf(texture, 0x84FE, desc.anisotropy.max(1) as f32);
            match desc.compare {
                Some(func) => {
                    gl::TextureParameteri(texture, gl::TEXTURE_COMPARE_MODE, gl::COMPARE_REF_TO_TEXTURE as GLint);
                    gl::TextureParameteri(texture, gl::TEXTURE_COMPARE_FUNC, compare(func) as GLint);
                }
                None => gl::TextureParameteri(texture, gl::TEXTURE_COMPARE_MODE, gl::NONE as GLint),
            }
        }
    }

    fn apply_state(&self, desc: &PipelineDesc) {
        unsafe {
            set_enabled(gl::DEPTH_TEST, desc.depth.test);
            gl::DepthFunc(compare(desc.depth.func));
            gl::DepthMask(if desc.depth.write { gl::TRUE } else { gl::FALSE });
            match desc.blend {
                Some(blend) => {
                    gl::Enable(gl::BLEND);
                    gl::BlendEquation(gl::FUNC_ADD);
                    gl::BlendFunc(blend_factor(blend.src), blend_factor(blend.dst));
                }
                None => gl::Disable(gl::BLEND),
            }
            match desc.cull {
                Cull::None => gl::Disable(gl::CULL_FACE),
                Cull::Back => {
                    gl::Enable(gl::CULL_FACE);
                    gl::CullFace(gl::BACK);
                }
                Cull::Front => {
                    gl::Enable(gl::CULL_FACE);
                    gl::CullFace(gl::FRONT);
                }
            }
            gl::FrontFace(gl::CCW);
            let mask = if desc.color_write { gl::TRUE } else { gl::FALSE };
            gl::ColorMask(mask, mask, mask, mask);
            self.stencil.set(desc.stencil);
            self.apply_stencil();
            let wireframe = super::wireframe();
            let line = match desc.polygon {
                PolygonMode::Line => true,
                PolygonMode::Solid => false,
                PolygonMode::Fill => wireframe,
            };
            if line != self.wireframe.get() {
                self.wireframe.set(line);
            }
            gl::PolygonMode(gl::FRONT_AND_BACK, if line { gl::LINE } else { gl::FILL });
        }
    }

    fn apply_stencil(&self) {
        unsafe {
            match self.stencil.get() {
                Some(s) => {
                    gl::Enable(gl::STENCIL_TEST);
                    gl::StencilFunc(compare(s.func), self.stencil_reference.get() as GLint, s.read_mask as GLuint);
                    gl::StencilOp(stencil_op(s.fail), stencil_op(s.depth_fail), stencil_op(s.pass));
                    gl::StencilMask(s.write_mask as GLuint);
                }
                None => gl::Disable(gl::STENCIL_TEST),
            }
        }
    }
}

fn framebuffer_of(target: TargetRef) -> GLuint {
    match target {
        TargetRef::Window => 0,
        TargetRef::Target(raw) => raw as GLuint,
    }
}

unsafe fn set_enabled(cap: GLenum, on: bool) {
    if on {
        gl::Enable(cap)
    } else {
        gl::Disable(cap)
    }
}

fn filter(f: Filter) -> GLenum {
    match f {
        Filter::Nearest => gl::NEAREST,
        Filter::Linear => gl::LINEAR,
    }
}

fn wrap(w: Wrap) -> GLenum {
    match w {
        Wrap::Repeat => gl::REPEAT,
        Wrap::ClampToEdge => gl::CLAMP_TO_EDGE,
        Wrap::ClampToBorder => gl::CLAMP_TO_BORDER,
    }
}

fn compare(f: CompareFunc) -> GLenum {
    match f {
        CompareFunc::Never => gl::NEVER,
        CompareFunc::Less => gl::LESS,
        CompareFunc::Equal => gl::EQUAL,
        CompareFunc::LessEqual => gl::LEQUAL,
        CompareFunc::Greater => gl::GREATER,
        CompareFunc::NotEqual => gl::NOTEQUAL,
        CompareFunc::GreaterEqual => gl::GEQUAL,
        CompareFunc::Always => gl::ALWAYS,
    }
}

fn blend_factor(f: BlendFactor) -> GLenum {
    match f {
        BlendFactor::Zero => gl::ZERO,
        BlendFactor::One => gl::ONE,
        BlendFactor::SrcAlpha => gl::SRC_ALPHA,
        BlendFactor::OneMinusSrcAlpha => gl::ONE_MINUS_SRC_ALPHA,
        BlendFactor::DstAlpha => gl::DST_ALPHA,
        BlendFactor::OneMinusDstAlpha => gl::ONE_MINUS_DST_ALPHA,
        BlendFactor::SrcColor => gl::SRC_COLOR,
        BlendFactor::OneMinusSrcColor => gl::ONE_MINUS_SRC_COLOR,
    }
}

fn stencil_op(op: StencilOp) -> GLenum {
    match op {
        StencilOp::Keep => gl::KEEP,
        StencilOp::Zero => gl::ZERO,
        StencilOp::Replace => gl::REPLACE,
        StencilOp::IncrementClamp => gl::INCR,
        StencilOp::DecrementClamp => gl::DECR,
    }
}

fn topology(t: Topology) -> GLenum {
    match t {
        Topology::Triangles => gl::TRIANGLES,
        Topology::TriangleStrip => gl::TRIANGLE_STRIP,
        Topology::Lines => gl::LINES,
    }
}

fn texture_target(desc: &TextureDesc) -> GLenum {
    match desc.kind {
        TextureKind::D2 if desc.samples > 1 => gl::TEXTURE_2D_MULTISAMPLE,
        TextureKind::D2 => gl::TEXTURE_2D,
        TextureKind::D2Array => gl::TEXTURE_2D_ARRAY,
        TextureKind::D3 => gl::TEXTURE_3D,
        TextureKind::CubeArray => gl::TEXTURE_CUBE_MAP_ARRAY,
    }
}

fn internal_format(format: Format) -> GLenum {
    match format {
        Format::R8 => gl::R8,
        Format::R16 => gl::R16,
        Format::R16F => gl::R16F,
        Format::R32F => gl::R32F,
        Format::RG16F => gl::RG16F,
        Format::Rgba8 => gl::RGBA8,
        Format::Srgba8 => gl::SRGB8_ALPHA8,
        Format::Rgba16F => gl::RGBA16F,
        Format::Rg11B10F => gl::R11F_G11F_B10F,
        Format::Depth16 => gl::DEPTH_COMPONENT16,
        Format::Depth24 => gl::DEPTH_COMPONENT24,
        Format::Depth32F => gl::DEPTH_COMPONENT32F,
        Format::Depth24Stencil8 => gl::DEPTH24_STENCIL8,
    }
}

fn upload_format(format: UploadFormat) -> (GLenum, GLenum) {
    match format {
        UploadFormat::R8 => (gl::RED, gl::UNSIGNED_BYTE),
        UploadFormat::R16 => (gl::RED, gl::UNSIGNED_SHORT),
        UploadFormat::Rgba8 => (gl::RGBA, gl::UNSIGNED_BYTE),
        UploadFormat::R32F => (gl::RED, gl::FLOAT),
        UploadFormat::Rg32F => (gl::RG, gl::FLOAT),
        UploadFormat::Rgba32F => (gl::RGBA, gl::FLOAT),
    }
}

impl Device for GlDevice {
    fn kind(&self) -> BackendKind {
        BackendKind::OpenGl
    }

    fn describe(&self) -> String {
        unsafe {
            let text = |name| {
                let p = gl::GetString(name);
                if p.is_null() {
                    String::new()
                } else {
                    std::ffi::CStr::from_ptr(p as *const _).to_string_lossy().into_owned()
                }
            };
            format!("{}, {}", text(gl::RENDERER), text(gl::VERSION))
        }
    }

    /// From NVIDIA's `GL_NVX_gpu_memory_info`, or approximated from
    /// `GL_ATI_meminfo` (AMD/Mesa), which only reports free memory.
    fn video_memory(&self) -> Option<(u64, u64)> {
        const NVX_TOTAL: u32 = 0x9048;
        const NVX_AVAILABLE: u32 = 0x9049;
        const ATI_TEXTURE_FREE: u32 = 0x87FC;
        let has_extension = |name: &str| {
            let mut count = 0;
            unsafe { gl::GetIntegerv(gl::NUM_EXTENSIONS, &mut count) };
            (0..count.max(0) as u32).any(|i| unsafe {
                let ptr = gl::GetStringi(gl::EXTENSIONS, i);
                !ptr.is_null() && std::ffi::CStr::from_ptr(ptr as *const _).to_bytes() == name.as_bytes()
            })
        };
        if has_extension("GL_NVX_gpu_memory_info") {
            let (mut total, mut available) = (0, 0);
            unsafe {
                gl::GetIntegerv(NVX_TOTAL, &mut total);
                gl::GetIntegerv(NVX_AVAILABLE, &mut available);
            }
            // Drivers that only pretend to have it (Mesa's llvmpipe) answer -1.
            let total = total.max(0) as u64 * 1024;
            let available = available.max(0) as u64 * 1024;
            return Some((total.saturating_sub(available), total));
        }
        if has_extension("GL_ATI_meminfo") {
            let mut info = [0i32; 4];
            unsafe { gl::GetIntegerv(ATI_TEXTURE_FREE, info.as_mut_ptr()) };
            // Free memory only: shown as "0 used of the free".
            return Some((0, info[0].max(0) as u64 * 1024));
        }
        None
    }

    fn create_buffer(&self, _usage: BufferUsage, memory: BufferMemory, size: usize, data: Option<&[u8]>) -> RawId {
        let mut buffer = 0;
        unsafe {
            gl::CreateBuffers(1, &mut buffer);
            let hint = match memory {
                BufferMemory::Static => gl::STATIC_DRAW,
                BufferMemory::Dynamic => gl::DYNAMIC_DRAW,
            };
            let pointer = data.map_or(ptr::null(), |d| d.as_ptr() as *const GLvoid);
            gl::NamedBufferData(buffer, size as GLsizeiptr, pointer, hint);
        }
        buffer as RawId
    }

    fn write_buffer(&self, buffer: RawId, offset: usize, data: &[u8], orphan: bool) {
        if data.is_empty() {
            return;
        }
        unsafe {
            if orphan {
                let mut size: GLint64 = 0;
                gl::GetNamedBufferParameteri64v(buffer as GLuint, gl::BUFFER_SIZE, &mut size);
                let mut usage: GLint = 0;
                gl::GetNamedBufferParameteriv(buffer as GLuint, gl::BUFFER_USAGE, &mut usage);
                gl::NamedBufferData(buffer as GLuint, size as GLsizeiptr, ptr::null(), usage as GLenum);
            }
            gl::NamedBufferSubData(buffer as GLuint, offset as GLintptr, data.len() as GLsizeiptr, data.as_ptr() as *const GLvoid);
        }
    }

    fn resize_buffer(&self, buffer: RawId, size: usize) {
        unsafe {
            let mut usage: GLint = 0;
            gl::GetNamedBufferParameteriv(buffer as GLuint, gl::BUFFER_USAGE, &mut usage);
            gl::NamedBufferData(buffer as GLuint, size as GLsizeiptr, ptr::null(), usage as GLenum);
        }
    }

    fn create_texture(&self, desc: &TextureDesc) -> RawId {
        let target = texture_target(desc);
        let format = internal_format(desc.format);
        let (w, h) = (desc.width.max(1) as GLsizei, desc.height.max(1) as GLsizei);
        let mut texture = 0;
        unsafe {
            gl::CreateTextures(target, 1, &mut texture);
            match desc.kind {
                TextureKind::D2 if desc.samples > 1 => {
                    gl::TextureStorage2DMultisample(texture, desc.samples as GLsizei, format, w, h, gl::TRUE)
                }
                TextureKind::D2 => gl::TextureStorage2D(texture, desc.mip_count() as GLsizei, format, w, h),
                TextureKind::D2Array | TextureKind::D3 | TextureKind::CubeArray => {
                    gl::TextureStorage3D(texture, desc.mip_count() as GLsizei, format, w, h, desc.layers.max(1) as GLsizei)
                }
            }
            if desc.samples <= 1 {
                // Until a sampler is applied: GL's default min filter
                // needs mipmaps a single-level texture does not have.
                let min = if desc.mip_count() > 1 { gl::LINEAR_MIPMAP_LINEAR } else { gl::LINEAR };
                gl::TextureParameteri(texture, gl::TEXTURE_MIN_FILTER, min as GLint);
                gl::TextureParameteri(texture, gl::TEXTURE_MAX_LEVEL, desc.mip_count() as GLint - 1);
            }
        }
        texture as RawId
    }

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
    ) {
        let (pixel_format, pixel_type) = upload_format(format);
        let alignment = match format.texel_size() {
            1 => 1,
            2 => 2,
            _ => 4,
        };
        unsafe {
            gl::PixelStorei(gl::UNPACK_ALIGNMENT, alignment);
            let pointer = data.as_ptr() as *const GLvoid;
            match desc.kind {
                TextureKind::D2 => gl::TextureSubImage2D(
                    texture as GLuint,
                    mip as GLint,
                    region.x,
                    region.y,
                    region.width as GLsizei,
                    region.height as GLsizei,
                    pixel_format,
                    pixel_type,
                    pointer,
                ),
                _ => gl::TextureSubImage3D(
                    texture as GLuint,
                    mip as GLint,
                    region.x,
                    region.y,
                    layer as GLint,
                    region.width as GLsizei,
                    region.height as GLsizei,
                    layers as GLsizei,
                    pixel_format,
                    pixel_type,
                    pointer,
                ),
            }
            gl::PixelStorei(gl::UNPACK_ALIGNMENT, 4);
        }
    }

    fn generate_mips(&self, texture: RawId, _desc: &TextureDesc) {
        unsafe { gl::GenerateTextureMipmap(texture as GLuint) };
    }

    fn create_sampler(&self, desc: &SamplerDesc) -> RawId {
        let mut samplers = self.samplers.borrow_mut();
        samplers.push(*desc);
        samplers.len() as RawId
    }

    fn create_target(&self, colors: &[Attachment], depth: Option<Attachment>) -> RawId {
        let mut framebuffer = 0;
        unsafe {
            gl::CreateFramebuffers(1, &mut framebuffer);
            let attach = |point: GLenum, a: &Attachment| match a.layer {
                Some(layer) => gl::NamedFramebufferTextureLayer(framebuffer, point, a.texture as GLuint, a.mip as GLint, layer as GLint),
                None => gl::NamedFramebufferTexture(framebuffer, point, a.texture as GLuint, a.mip as GLint),
            };
            for (i, color) in colors.iter().enumerate() {
                attach(gl::COLOR_ATTACHMENT0 + i as GLenum, color);
            }
            if let Some(depth) = depth {
                let point = if depth.desc.format.has_stencil() { gl::DEPTH_STENCIL_ATTACHMENT } else { gl::DEPTH_ATTACHMENT };
                attach(point, &depth);
            }
            if colors.is_empty() {
                gl::NamedFramebufferDrawBuffer(framebuffer, gl::NONE);
                gl::NamedFramebufferReadBuffer(framebuffer, gl::NONE);
            } else {
                let buffers: Vec<GLenum> = (0..colors.len()).map(|i| gl::COLOR_ATTACHMENT0 + i as GLenum).collect();
                gl::NamedFramebufferDrawBuffers(framebuffer, buffers.len() as GLsizei, buffers.as_ptr());
                gl::NamedFramebufferReadBuffer(framebuffer, gl::COLOR_ATTACHMENT0);
            }
            let status = gl::CheckNamedFramebufferStatus(framebuffer, gl::FRAMEBUFFER);
            if status != gl::FRAMEBUFFER_COMPLETE {
                log::error!("render target incomplete (status 0x{status:x})");
            }
        }
        self.targets.borrow_mut().insert(framebuffer, TargetInfo { colors: colors.len() });
        framebuffer as RawId
    }

    fn create_program(&self, vertex: &str, fragment: &str, label: &str) -> Result<RawId, String> {
        unsafe {
            let vs = compile_stage(gl::VERTEX_SHADER, vertex, label)?;
            let fs = match compile_stage(gl::FRAGMENT_SHADER, fragment, label) {
                Ok(fs) => fs,
                Err(e) => {
                    gl::DeleteShader(vs);
                    return Err(e);
                }
            };
            let program = gl::CreateProgram();
            gl::AttachShader(program, vs);
            gl::AttachShader(program, fs);
            gl::LinkProgram(program);
            gl::DeleteShader(vs);
            gl::DeleteShader(fs);
            let mut success = gl::FALSE as GLint;
            gl::GetProgramiv(program, gl::LINK_STATUS, &mut success);
            if success != gl::TRUE as GLint {
                let log = program_log(program);
                gl::DeleteProgram(program);
                return Err(format!("shader program ({label}) failed to link:\n{}", log.trim_end()));
            }
            Ok(program as RawId)
        }
    }

    fn create_pipeline(&self, program: RawId, desc: &PipelineDesc) -> RawId {
        let vao = self.vertex_array(&desc.vertex_layouts);
        let mut pipelines = self.pipelines.borrow_mut();
        pipelines.push(GlPipeline { program: program as GLuint, desc: desc.clone(), vao });
        pipelines.len() as RawId
    }

    fn create_queries(&self, count: u32) -> RawId {
        let mut names = vec![0; count as usize];
        unsafe { gl::CreateQueries(gl::TIMESTAMP, count as GLsizei, names.as_mut_ptr()) };
        let mut queries = self.queries.borrow_mut();
        queries.push(names);
        queries.len() as RawId
    }

    fn write_timestamp(&self, queries: RawId, index: u32) {
        if let Some(&name) = self.queries.borrow().get(queries as usize - 1).and_then(|q| q.get(index as usize)) {
            unsafe { gl::QueryCounter(name, gl::TIMESTAMP) };
        }
    }

    fn read_timestamps(&self, queries: RawId, count: u32) -> Option<Vec<u64>> {
        let all = self.queries.borrow();
        let names = all.get(queries as usize - 1)?;
        let names = &names[..(count as usize).min(names.len())];
        unsafe {
            let mut available = 0;
            gl::GetQueryObjectiv(*names.last()?, gl::QUERY_RESULT_AVAILABLE, &mut available);
            if available == 0 {
                return None;
            }
            Some(
                names
                    .iter()
                    .map(|&name| {
                        let mut value = 0u64;
                        gl::GetQueryObjectui64v(name, gl::QUERY_RESULT, &mut value);
                        value
                    })
                    .collect(),
            )
        }
    }

    fn destroy(&self, raw: Raw) {
        unsafe {
            match raw {
                Raw::Buffer(id) => gl::DeleteBuffers(1, &(id as GLuint)),
                Raw::Texture(id) => {
                    self.texture_samplers.borrow_mut().remove(&(id as GLuint));
                    gl::DeleteTextures(1, &(id as GLuint));
                }
                Raw::Sampler(_) => {}
                Raw::Target(id) => {
                    self.targets.borrow_mut().remove(&(id as GLuint));
                    crate::core::renderer::framebuffer::delete(&[id as GLuint]);
                }
                Raw::Program(id) => gl::DeleteProgram(id as GLuint),
                // Pipelines are only state; the slot stays (ids are indices).
                Raw::Pipeline(_) => {}
                Raw::Queries(id) => {
                    if let Some(names) = self.queries.borrow_mut().get_mut(id as usize - 1) {
                        gl::DeleteQueries(names.len() as GLsizei, names.as_ptr());
                        names.clear();
                    }
                }
            }
        }
    }

    fn begin_pass(&self, target: TargetRef, size: (u32, u32), color: Load, depth: Load) {
        self.bind_framebuffer(target);
        unsafe {
            gl::Viewport(0, 0, size.0 as GLsizei, size.1 as GLsizei);
            gl::Disable(gl::SCISSOR_TEST);
            let mut mask = 0;
            if let Load::Color(c) = color {
                gl::ColorMask(gl::TRUE, gl::TRUE, gl::TRUE, gl::TRUE);
                gl::ClearColor(c[0], c[1], c[2], c[3]);
                mask |= gl::COLOR_BUFFER_BIT;
            }
            if let Load::Depth(d) = depth {
                gl::DepthMask(gl::TRUE);
                gl::ClearDepth(d as f64);
                gl::StencilMask(0xFF);
                gl::ClearStencil(0);
                mask |= gl::DEPTH_BUFFER_BIT | gl::STENCIL_BUFFER_BIT;
            }
            if mask != 0 {
                gl::Clear(mask);
            }
        }
    }

    fn resume_pass(&self, target: TargetRef, size: (u32, u32)) {
        self.bind_framebuffer(target);
        unsafe { gl::Viewport(0, 0, size.0 as GLsizei, size.1 as GLsizei) };
    }

    fn end_pass(&self) {}

    fn set_viewport(&self, rect: Rect) {
        unsafe { gl::Viewport(rect.x, rect.y, rect.width as GLsizei, rect.height as GLsizei) };
    }

    fn set_scissor(&self, rect: Option<Rect>) {
        unsafe {
            match rect {
                Some(r) => {
                    gl::Enable(gl::SCISSOR_TEST);
                    gl::Scissor(r.x, r.y, r.width as GLsizei, r.height as GLsizei);
                }
                None => gl::Disable(gl::SCISSOR_TEST),
            }
        }
    }

    fn set_pipeline(&self, pipeline: RawId, _program: RawId) {
        let pipelines = self.pipelines.borrow();
        let Some(p) = pipeline.checked_sub(1).and_then(|i| pipelines.get(i as usize)) else {
            return;
        };
        unsafe {
            gl::UseProgram(p.program);
            gl::BindVertexArray(p.vao);
        }
        self.current_vao.set(p.vao);
        self.current_topology.set(topology(p.desc.topology));
        self.stencil_reference.set(super::stencil_reference());
        self.apply_state(&super::resolve_pipeline(&p.desc));
    }

    fn set_stencil_reference(&self, reference: u8) {
        self.stencil_reference.set(reference);
        self.apply_stencil();
    }

    fn bind_texture(&self, slot: u32, texture: RawId, _kind: TextureKind, sampler: RawId) {
        self.apply_sampler(texture as GLuint, sampler);
        unsafe { gl::BindTextureUnit(slot, texture as GLuint) };
    }

    fn unbind_texture(&self, slot: u32, _kind: TextureKind) {
        unsafe { gl::BindTextureUnit(slot, 0) };
    }

    fn bind_uniform_buffer(&self, slot: u32, buffer: RawId, offset: usize, size: usize) {
        unsafe { gl::BindBufferRange(gl::UNIFORM_BUFFER, slot, buffer as GLuint, offset as GLintptr, size as GLsizeiptr) };
    }

    fn bind_storage_buffer(&self, slot: u32, buffer: RawId, offset: usize, size: usize) {
        unsafe { gl::BindBufferRange(gl::SHADER_STORAGE_BUFFER, slot, buffer as GLuint, offset as GLintptr, size as GLsizeiptr) };
    }

    fn set_vertex_buffer(&self, slot: u32, buffer: RawId, offset: usize, stride: u32) {
        unsafe {
            gl::VertexArrayVertexBuffer(self.current_vao.get(), slot, buffer as GLuint, offset as GLintptr, stride as GLsizei);
        }
    }

    fn set_index_buffer(&self, buffer: RawId, format: IndexFormat) {
        self.index_format.set(match format {
            IndexFormat::U16 => gl::UNSIGNED_SHORT,
            IndexFormat::U32 => gl::UNSIGNED_INT,
        });
        unsafe { gl::VertexArrayElementBuffer(self.current_vao.get(), buffer as GLuint) };
    }

    fn draw(&self, first_vertex: u32, vertices: u32, first_instance: u32, instances: u32) {
        unsafe {
            gl::DrawArraysInstancedBaseInstance(
                self.current_topology.get(),
                first_vertex as GLint,
                vertices as GLsizei,
                instances as GLsizei,
                first_instance,
            );
        }
    }

    fn draw_indexed(&self, first_index: u32, indices: u32, base_vertex: i32, first_instance: u32, instances: u32) {
        let format = self.index_format.get();
        let index_size = if format == gl::UNSIGNED_SHORT { 2 } else { 4 };
        unsafe {
            gl::DrawElementsInstancedBaseVertexBaseInstance(
                self.current_topology.get(),
                indices as GLsizei,
                format,
                (first_index as usize * index_size) as *const GLvoid,
                instances as GLsizei,
                base_vertex,
                first_instance,
            );
        }
    }

    fn clear(&self, planes: Planes, color: [f32; 4], depth: f32, stencil: u8) {
        let mut mask = 0;
        unsafe {
            if planes.color {
                gl::ColorMask(gl::TRUE, gl::TRUE, gl::TRUE, gl::TRUE);
                gl::ClearColor(color[0], color[1], color[2], color[3]);
                mask |= gl::COLOR_BUFFER_BIT;
            }
            if planes.depth {
                gl::DepthMask(gl::TRUE);
                gl::ClearDepth(depth as f64);
                mask |= gl::DEPTH_BUFFER_BIT;
            }
            if planes.stencil {
                gl::StencilMask(0xFF);
                gl::ClearStencil(stencil as GLint);
                mask |= gl::STENCIL_BUFFER_BIT;
            }
            if mask != 0 {
                gl::Clear(mask);
            }
        }
    }

    fn blit(&self, src: TargetRef, src_rect: Rect, dst: TargetRef, dst_rect: Rect, planes: Planes, linear: bool) {
        let mut mask = 0;
        if planes.color {
            mask |= gl::COLOR_BUFFER_BIT;
        }
        if planes.depth {
            mask |= gl::DEPTH_BUFFER_BIT;
        }
        if planes.stencil {
            mask |= gl::STENCIL_BUFFER_BIT;
        }
        let filter = if linear && !planes.depth && !planes.stencil { gl::LINEAR } else { gl::NEAREST };
        unsafe {
            gl::BlitNamedFramebuffer(
                framebuffer_of(src),
                framebuffer_of(dst),
                src_rect.x,
                src_rect.y,
                src_rect.x + src_rect.width as GLint,
                src_rect.y + src_rect.height as GLint,
                dst_rect.x,
                dst_rect.y,
                dst_rect.x + dst_rect.width as GLint,
                dst_rect.y + dst_rect.height as GLint,
                mask,
                filter,
            );
        }
    }

    fn read_pixels(&self, target: TargetRef, rect: Rect) -> Vec<u8> {
        let mut pixels = vec![0u8; rect.width as usize * rect.height as usize * 4];
        unsafe {
            let framebuffer = framebuffer_of(target);
            gl::BindFramebuffer(gl::READ_FRAMEBUFFER, framebuffer);
            if framebuffer == 0 {
                gl::ReadBuffer(gl::BACK);
            } else if self.targets.borrow().get(&framebuffer).map_or(0, |t| t.colors) > 0 {
                gl::ReadBuffer(gl::COLOR_ATTACHMENT0);
            }
            gl::PixelStorei(gl::PACK_ALIGNMENT, 1);
            gl::ReadPixels(
                rect.x,
                rect.y,
                rect.width as GLsizei,
                rect.height as GLsizei,
                gl::RGBA,
                gl::UNSIGNED_BYTE,
                pixels.as_mut_ptr() as *mut GLvoid,
            );
            gl::PixelStorei(gl::PACK_ALIGNMENT, 4);
        }
        pixels
    }

    fn begin_frame(&self) {}

    fn end_frame(&self) {}
}

unsafe fn compile_stage(stage: GLenum, source: &str, label: &str) -> Result<GLuint, String> {
    let stage_name = if stage == gl::VERTEX_SHADER { "vertex" } else { "fragment" };
    let c_source = CString::new(source.as_bytes())
        .map_err(|_| format!("{stage_name} shader ({label}) contains a NUL byte"))?;
    let shader = gl::CreateShader(stage);
    gl::ShaderSource(shader, 1, &c_source.as_ptr(), ptr::null());
    gl::CompileShader(shader);
    let mut success = gl::FALSE as GLint;
    gl::GetShaderiv(shader, gl::COMPILE_STATUS, &mut success);
    if success == gl::TRUE as GLint {
        return Ok(shader);
    }
    let mut len: GLint = 0;
    gl::GetShaderiv(shader, gl::INFO_LOG_LENGTH, &mut len);
    let mut buffer = vec![0u8; len.max(1) as usize];
    let mut written: GLsizei = 0;
    gl::GetShaderInfoLog(shader, buffer.len() as GLsizei, &mut written, buffer.as_mut_ptr() as *mut GLchar);
    buffer.truncate(written.max(0) as usize);
    gl::DeleteShader(shader);
    Err(format!(
        "{stage_name} shader ({label}) failed to compile:\n{}",
        crate::core::renderer::shader::annotate_info_log(&String::from_utf8_lossy(&buffer), source)
    ))
}

unsafe fn program_log(program: GLuint) -> String {
    let mut len: GLint = 0;
    gl::GetProgramiv(program, gl::INFO_LOG_LENGTH, &mut len);
    let mut buffer = vec![0u8; len.max(1) as usize];
    let mut written: GLsizei = 0;
    gl::GetProgramInfoLog(program, buffer.len() as GLsizei, &mut written, buffer.as_mut_ptr() as *mut GLchar);
    buffer.truncate(written.max(0) as usize);
    String::from_utf8_lossy(&buffer).into_owned()
}
