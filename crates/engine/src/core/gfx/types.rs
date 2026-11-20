//! Descriptions of GPU resources and pipeline state, the same for every
//! backend.

/// Which API draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BackendKind {
    /// No GPU: the headless server, tests.
    Null,
    OpenGl,
    Vulkan,
}

impl BackendKind {
    pub fn name(self) -> &'static str {
        match self {
            BackendKind::Null => "none",
            BackendKind::OpenGl => "OpenGL",
            BackendKind::Vulkan => "Vulkan",
        }
    }

    /// From `--renderer`/`FERRITE_RENDERER` text: `gl`, `opengl`, `vulkan`, `vk`.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "gl" | "opengl" => Some(BackendKind::OpenGl),
            "vk" | "vulkan" => Some(BackendKind::Vulkan),
            _ => None,
        }
    }
}

/// Texel formats in use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    R8,
    /// 16 bit unsigned normalised.
    R16,
    R16F,
    R32F,
    RG16F,
    Rgba8,
    /// RGBA8 with sRGB colour, linear alpha.
    Srgba8,
    Rgba16F,
    Rg11B10F,
    Depth16,
    Depth24,
    Depth32F,
    Depth24Stencil8,
}

impl Format {
    pub fn is_depth(self) -> bool {
        matches!(
            self,
            Format::Depth16 | Format::Depth24 | Format::Depth32F | Format::Depth24Stencil8
        )
    }

    pub fn has_stencil(self) -> bool {
        self == Format::Depth24Stencil8
    }

    /// Bytes per texel (as uploaded for the colour formats).
    pub fn texel_size(self) -> usize {
        match self {
            Format::R8 => 1,
            Format::R16 | Format::R16F | Format::Depth16 => 2,
            Format::R32F | Format::RG16F | Format::Rgba8 | Format::Srgba8 => 4,
            Format::Rg11B10F | Format::Depth24 | Format::Depth32F | Format::Depth24Stencil8 => 4,
            Format::Rgba16F => 8,
        }
    }
}

/// Data handed to a texture upload, by component type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UploadFormat {
    /// One byte per texel (a red channel).
    R8,
    /// Two bytes per texel, little-endian u16 (a red channel).
    R16,
    /// Four bytes per texel.
    Rgba8,
    /// One f32 per texel.
    R32F,
    /// Two f32 per texel.
    Rg32F,
    /// Four f32 per texel.
    Rgba32F,
}

impl UploadFormat {
    pub fn texel_size(self) -> usize {
        match self {
            UploadFormat::R8 => 1,
            UploadFormat::R16 => 2,
            UploadFormat::Rgba8 | UploadFormat::R32F => 4,
            UploadFormat::Rg32F => 8,
            UploadFormat::Rgba32F => 16,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextureKind {
    D2,
    D2Array,
    D3,
    CubeArray,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextureDesc {
    pub kind: TextureKind,
    pub format: Format,
    pub width: u32,
    pub height: u32,
    /// Depth of a 3D texture; layers of an array (cubes × 6 for a cube
    /// array); 1 otherwise.
    pub layers: u32,
    /// Mip levels; 0 for the full chain.
    pub mips: u32,
    /// Samples per texel: 1, or more for a multisampled render target
    /// (which cannot be sampled, only resolved).
    pub samples: u32,
}

impl TextureDesc {
    pub fn d2(format: Format, width: u32, height: u32) -> Self {
        TextureDesc {
            kind: TextureKind::D2,
            format,
            width,
            height,
            layers: 1,
            mips: 1,
            samples: 1,
        }
    }

    pub fn with_mips(mut self, mips: u32) -> Self {
        self.mips = mips;
        self
    }

    pub fn with_samples(mut self, samples: u32) -> Self {
        self.samples = samples.max(1);
        self
    }

    /// The mip levels this texture really has.
    pub fn mip_count(&self) -> u32 {
        if self.mips != 0 {
            return self.mips;
        }
        let largest = match self.kind {
            TextureKind::D3 => self.width.max(self.height).max(self.layers),
            _ => self.width.max(self.height),
        };
        32 - largest.max(1).leading_zeros()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Filter {
    Nearest,
    Linear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Wrap {
    Repeat,
    ClampToEdge,
    ClampToBorder,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BorderColor {
    TransparentBlack,
    OpaqueBlack,
    OpaqueWhite,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CompareFunc {
    Never,
    Less,
    Equal,
    LessEqual,
    Greater,
    NotEqual,
    GreaterEqual,
    Always,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SamplerDesc {
    pub min: Filter,
    pub mag: Filter,
    /// Filter between mip levels; `None` samples level 0 only.
    pub mip: Option<Filter>,
    pub wrap: [Wrap; 3],
    pub border: BorderColor,
    /// Anisotropic filtering (1 = off).
    pub anisotropy: u8,
    /// A shadow sampler: compares against the reference.
    pub compare: Option<CompareFunc>,
}

impl SamplerDesc {
    pub const fn new(filter: Filter, wrap: Wrap) -> Self {
        SamplerDesc {
            min: filter,
            mag: filter,
            mip: None,
            wrap: [wrap; 3],
            border: BorderColor::TransparentBlack,
            anisotropy: 1,
            compare: None,
        }
    }

    pub const LINEAR_CLAMP: SamplerDesc = SamplerDesc::new(Filter::Linear, Wrap::ClampToEdge);
    pub const NEAREST_CLAMP: SamplerDesc = SamplerDesc::new(Filter::Nearest, Wrap::ClampToEdge);
    pub const LINEAR_REPEAT: SamplerDesc = SamplerDesc::new(Filter::Linear, Wrap::Repeat);

    pub const fn mipmapped(mut self) -> Self {
        self.mip = Some(Filter::Linear);
        self
    }

    pub const fn compare(mut self, func: CompareFunc) -> Self {
        self.compare = Some(func);
        self
    }

    pub const fn anisotropic(mut self, level: u8) -> Self {
        self.anisotropy = level;
        self
    }
}

/// What a buffer is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BufferUsage(pub u32);

impl BufferUsage {
    pub const VERTEX: BufferUsage = BufferUsage(1);
    pub const INDEX: BufferUsage = BufferUsage(2);
    pub const UNIFORM: BufferUsage = BufferUsage(4);
    pub const STORAGE: BufferUsage = BufferUsage(8);
    /// Geometry a ray tracing acceleration structure may be built from.
    pub const RAY_TRACING_INPUT: BufferUsage = BufferUsage(16);

    pub fn contains(self, other: BufferUsage) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for BufferUsage {
    type Output = BufferUsage;
    fn bitor(self, rhs: BufferUsage) -> BufferUsage {
        BufferUsage(self.0 | rhs.0)
    }
}

/// How often a buffer's contents change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BufferMemory {
    /// Written once (or rarely): device local.
    Static,
    /// Rewritten often (every frame or more), read a few times.
    Dynamic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VertexFormat {
    F32,
    F32x2,
    F32x3,
    F32x4,
    U32,
    U32x2,
    U32x3,
    U32x4,
}

impl VertexFormat {
    pub fn size(self) -> u32 {
        self.components() * 4
    }

    pub fn components(self) -> u32 {
        match self {
            VertexFormat::F32 | VertexFormat::U32 => 1,
            VertexFormat::F32x2 | VertexFormat::U32x2 => 2,
            VertexFormat::F32x3 | VertexFormat::U32x3 => 3,
            VertexFormat::F32x4 | VertexFormat::U32x4 => 4,
        }
    }

    pub fn is_integer(self) -> bool {
        matches!(
            self,
            VertexFormat::U32 | VertexFormat::U32x2 | VertexFormat::U32x3 | VertexFormat::U32x4
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StepMode {
    Vertex,
    Instance,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VertexAttribute {
    pub location: u32,
    pub format: VertexFormat,
    pub offset: u32,
}

/// One vertex buffer slot of a pipeline. Its attributes are interned
/// (made once per distinct list, kept for the program's life), so a
/// layout is a small copyable value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VertexLayout {
    pub stride: u32,
    pub step: StepMode,
    pub attributes: &'static [VertexAttribute],
}

impl Default for VertexLayout {
    fn default() -> Self {
        VertexLayout { stride: 0, step: StepMode::Vertex, attributes: &[] }
    }
}

/// The interned copy of `attributes` (per thread: no lock on the draw
/// path; a thread interns a list once).
fn intern(attributes: &[VertexAttribute]) -> &'static [VertexAttribute] {
    use std::cell::RefCell;
    thread_local! {
        static INTERNED: RefCell<super::fx::FxHashMap<Vec<VertexAttribute>, &'static [VertexAttribute]>> =
            RefCell::new(Default::default());
    }
    INTERNED.with(|interned| {
        if let Some(&found) = interned.borrow().get(attributes) {
            return found;
        }
        let leaked: &'static [VertexAttribute] = Box::leak(attributes.to_vec().into_boxed_slice());
        interned.borrow_mut().insert(attributes.to_vec(), leaked);
        leaked
    })
}

impl VertexLayout {
    pub fn new(stride: u32, step: StepMode, attributes: &[VertexAttribute]) -> Self {
        VertexLayout { stride, step, attributes: intern(attributes) }
    }

    /// Interleaved attributes at consecutive locations from `first`, in
    /// the order given, packed without gaps.
    pub fn packed(first: u32, step: StepMode, formats: &[VertexFormat]) -> Self {
        let mut offset = 0;
        let mut attributes = [VertexAttribute { location: 0, format: VertexFormat::F32, offset: 0 }; 16];
        let n = formats.len().min(16);
        for (i, &format) in formats.iter().take(n).enumerate() {
            attributes[i] = VertexAttribute { location: first + i as u32, format, offset };
            offset += format.size();
        }
        VertexLayout::new(offset, step, &attributes[..n])
    }
}

/// The vertex buffer slots of a pipeline (at most `Layouts::MAX`), in slot
/// order; derefs to a slice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Layouts {
    len: u8,
    items: [VertexLayout; Layouts::MAX],
}

impl Layouts {
    pub const MAX: usize = 4;

    pub fn of(layouts: &[VertexLayout]) -> Self {
        assert!(layouts.len() <= Self::MAX, "at most {} vertex buffer slots", Self::MAX);
        let mut items = [VertexLayout::default(); Self::MAX];
        items[..layouts.len()].copy_from_slice(layouts);
        Layouts { len: layouts.len() as u8, items }
    }
}

impl std::ops::Deref for Layouts {
    type Target = [VertexLayout];
    fn deref(&self) -> &[VertexLayout] {
        &self.items[..self.len as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Topology {
    Triangles,
    TriangleStrip,
    Lines,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cull {
    None,
    Back,
    Front,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PolygonMode {
    /// Filled, or outlined while the wireframe debug view is on.
    Fill,
    /// Always filled (text and other 2D drawing).
    Solid,
    Line,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DepthState {
    pub test: bool,
    pub write: bool,
    pub func: CompareFunc,
}

impl DepthState {
    pub const OFF: DepthState = DepthState { test: false, write: false, func: CompareFunc::Always };
    pub const LESS: DepthState = DepthState { test: true, write: true, func: CompareFunc::Less };
    pub const LEQUAL: DepthState = DepthState { test: true, write: true, func: CompareFunc::LessEqual };
    /// Tested (`LESS`), not written.
    pub const READ: DepthState = DepthState { test: true, write: false, func: CompareFunc::Less };

    pub const fn func(mut self, func: CompareFunc) -> Self {
        self.func = func;
        self
    }

    pub const fn write(mut self, write: bool) -> Self {
        self.write = write;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BlendFactor {
    Zero,
    One,
    SrcAlpha,
    OneMinusSrcAlpha,
    DstAlpha,
    OneMinusDstAlpha,
    SrcColor,
    OneMinusSrcColor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlendState {
    pub src: BlendFactor,
    pub dst: BlendFactor,
}

impl BlendState {
    /// Straight alpha: `src·a + dst·(1−a)`.
    pub const ALPHA: BlendState =
        BlendState { src: BlendFactor::SrcAlpha, dst: BlendFactor::OneMinusSrcAlpha };
    /// Premultiplied alpha.
    pub const PREMULTIPLIED: BlendState =
        BlendState { src: BlendFactor::One, dst: BlendFactor::OneMinusSrcAlpha };
    pub const ADDITIVE: BlendState = BlendState { src: BlendFactor::One, dst: BlendFactor::One };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StencilOp {
    Keep,
    Zero,
    Replace,
    IncrementClamp,
    DecrementClamp,
}

/// The same for front and back faces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StencilState {
    pub func: CompareFunc,
    /// Pass (stencil and depth).
    pub pass: StencilOp,
    pub fail: StencilOp,
    pub depth_fail: StencilOp,
    pub read_mask: u8,
    pub write_mask: u8,
}

/// Fixed-function state of a draw, with the program. Hashed: pipelines
/// are looked up by it. A small copyable value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PipelineDesc {
    pub vertex_layouts: Layouts,
    pub topology: Topology,
    pub depth: DepthState,
    pub blend: Option<BlendState>,
    pub cull: Cull,
    /// Colour attachments written at all (`glColorMask`).
    pub color_write: bool,
    pub stencil: Option<StencilState>,
    pub polygon: PolygonMode,
}

impl Default for PipelineDesc {
    fn default() -> Self {
        PipelineDesc {
            vertex_layouts: Layouts::default(),
            topology: Topology::Triangles,
            depth: DepthState::LESS,
            blend: None,
            cull: Cull::None,
            color_write: true,
            stencil: None,
            polygon: PolygonMode::Fill,
        }
    }
}

/// What a pass does with an attachment's old contents.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Load {
    Keep,
    /// Clear colour to this.
    Color([f32; 4]),
    /// Clear depth (and stencil to 0) to this.
    Depth(f32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IndexFormat {
    U16,
    U32,
}

/// Which of a target's planes a blit or clear touches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Planes {
    pub color: bool,
    pub depth: bool,
    pub stencil: bool,
}

impl Planes {
    pub const COLOR: Planes = Planes { color: true, depth: false, stencil: false };
    pub const DEPTH: Planes = Planes { color: false, depth: true, stencil: false };
    pub const COLOR_DEPTH: Planes = Planes { color: true, depth: true, stencil: false };
    pub const ALL: Planes = Planes { color: true, depth: true, stencil: true };
}

/// A rectangle in a target, in texels from its first row (NDC y = −1,
/// GL's bottom-left).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Rect { x, y, width, height }
    }

    pub fn sized(width: u32, height: u32) -> Self {
        Rect { x: 0, y: 0, width, height }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_mip_chain_goes_down_to_one_texel() {
        assert_eq!(TextureDesc::d2(Format::Rgba8, 1024, 256).with_mips(0).mip_count(), 11);
        assert_eq!(TextureDesc::d2(Format::Rgba8, 1, 1).with_mips(0).mip_count(), 1);
        assert_eq!(TextureDesc::d2(Format::Rgba8, 300, 2).with_mips(0).mip_count(), 9);
    }

    #[test]
    fn packed_layouts_lay_attributes_end_to_end() {
        let layout = VertexLayout::packed(
            3,
            StepMode::Instance,
            &[VertexFormat::F32x4, VertexFormat::F32, VertexFormat::U32x4],
        );
        assert_eq!(layout.stride, 36);
        let offsets: Vec<(u32, u32)> = layout.attributes.iter().map(|a| (a.location, a.offset)).collect();
        assert_eq!(offsets, [(3, 0), (4, 16), (5, 20)]);
    }

    #[test]
    fn backend_names_parse() {
        assert_eq!(BackendKind::parse("Vulkan"), Some(BackendKind::Vulkan));
        assert_eq!(BackendKind::parse(" gl "), Some(BackendKind::OpenGl));
        assert_eq!(BackendKind::parse("metal"), None);
    }
}

/// A value for a uniform set by name (see `Device::set_uniform`).
#[derive(Clone, Copy, Debug)]
pub enum UniformValue<'a> {
    Int(i32),
    Int2([i32; 2]),
    Float(f32),
    Float2([f32; 2]),
    Float3([f32; 3]),
    Float4([f32; 4]),
    /// Column major.
    Mat4(&'a [f32; 16]),
    Floats(&'a [f32]),
    Mat4s(&'a [[f32; 16]]),
}
