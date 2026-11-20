//! `gfx` descriptions in Vulkan's terms.

use ash::vk;

use super::super::*;

pub fn format(f: Format, depth_stencil: vk::Format) -> vk::Format {
    match f {
        Format::R8 => vk::Format::R8_UNORM,
        Format::R16 => vk::Format::R16_UNORM,
        Format::R16F => vk::Format::R16_SFLOAT,
        Format::R32F => vk::Format::R32_SFLOAT,
        Format::RG16F => vk::Format::R16G16_SFLOAT,
        Format::Rgba8 => vk::Format::R8G8B8A8_UNORM,
        Format::Srgba8 => vk::Format::R8G8B8A8_SRGB,
        Format::Rgba16F => vk::Format::R16G16B16A16_SFLOAT,
        Format::Rg11B10F => vk::Format::B10G11R11_UFLOAT_PACK32,
        Format::Depth16 => vk::Format::D16_UNORM,
        Format::Depth24 | Format::Depth32F => vk::Format::D32_SFLOAT,
        Format::Depth24Stencil8 => depth_stencil,
    }
}

/// All aspects of an image of `f`.
pub fn aspects(f: Format) -> vk::ImageAspectFlags {
    if f.has_stencil() {
        vk::ImageAspectFlags::DEPTH | vk::ImageAspectFlags::STENCIL
    } else if f.is_depth() {
        vk::ImageAspectFlags::DEPTH
    } else {
        vk::ImageAspectFlags::COLOR
    }
}

/// The aspect sampled (depth only for depth formats).
pub fn sampled_aspect(f: Format) -> vk::ImageAspectFlags {
    if f.is_depth() {
        vk::ImageAspectFlags::DEPTH
    } else {
        vk::ImageAspectFlags::COLOR
    }
}

pub fn samples(n: u32) -> vk::SampleCountFlags {
    match n {
        0 | 1 => vk::SampleCountFlags::TYPE_1,
        2 => vk::SampleCountFlags::TYPE_2,
        3 | 4 => vk::SampleCountFlags::TYPE_4,
        5..=8 => vk::SampleCountFlags::TYPE_8,
        9..=16 => vk::SampleCountFlags::TYPE_16,
        _ => vk::SampleCountFlags::TYPE_32,
    }
}

pub fn filter(f: Filter) -> vk::Filter {
    match f {
        Filter::Nearest => vk::Filter::NEAREST,
        Filter::Linear => vk::Filter::LINEAR,
    }
}

pub fn wrap(w: Wrap) -> vk::SamplerAddressMode {
    match w {
        Wrap::Repeat => vk::SamplerAddressMode::REPEAT,
        Wrap::ClampToEdge => vk::SamplerAddressMode::CLAMP_TO_EDGE,
        Wrap::ClampToBorder => vk::SamplerAddressMode::CLAMP_TO_BORDER,
    }
}

pub fn compare(f: CompareFunc) -> vk::CompareOp {
    match f {
        CompareFunc::Never => vk::CompareOp::NEVER,
        CompareFunc::Less => vk::CompareOp::LESS,
        CompareFunc::Equal => vk::CompareOp::EQUAL,
        CompareFunc::LessEqual => vk::CompareOp::LESS_OR_EQUAL,
        CompareFunc::Greater => vk::CompareOp::GREATER,
        CompareFunc::NotEqual => vk::CompareOp::NOT_EQUAL,
        CompareFunc::GreaterEqual => vk::CompareOp::GREATER_OR_EQUAL,
        CompareFunc::Always => vk::CompareOp::ALWAYS,
    }
}

pub fn blend_factor(f: BlendFactor) -> vk::BlendFactor {
    match f {
        BlendFactor::Zero => vk::BlendFactor::ZERO,
        BlendFactor::One => vk::BlendFactor::ONE,
        BlendFactor::SrcAlpha => vk::BlendFactor::SRC_ALPHA,
        BlendFactor::OneMinusSrcAlpha => vk::BlendFactor::ONE_MINUS_SRC_ALPHA,
        BlendFactor::DstAlpha => vk::BlendFactor::DST_ALPHA,
        BlendFactor::OneMinusDstAlpha => vk::BlendFactor::ONE_MINUS_DST_ALPHA,
        BlendFactor::SrcColor => vk::BlendFactor::SRC_COLOR,
        BlendFactor::OneMinusSrcColor => vk::BlendFactor::ONE_MINUS_SRC_COLOR,
    }
}

pub fn stencil_op(op: StencilOp) -> vk::StencilOp {
    match op {
        StencilOp::Keep => vk::StencilOp::KEEP,
        StencilOp::Zero => vk::StencilOp::ZERO,
        StencilOp::Replace => vk::StencilOp::REPLACE,
        StencilOp::IncrementClamp => vk::StencilOp::INCREMENT_AND_CLAMP,
        StencilOp::DecrementClamp => vk::StencilOp::DECREMENT_AND_CLAMP,
    }
}

pub fn topology(t: Topology) -> vk::PrimitiveTopology {
    match t {
        Topology::Triangles => vk::PrimitiveTopology::TRIANGLE_LIST,
        Topology::TriangleStrip => vk::PrimitiveTopology::TRIANGLE_STRIP,
        Topology::Lines => vk::PrimitiveTopology::LINE_LIST,
    }
}

pub fn vertex_format(f: VertexFormat) -> vk::Format {
    match f {
        VertexFormat::F32 => vk::Format::R32_SFLOAT,
        VertexFormat::F32x2 => vk::Format::R32G32_SFLOAT,
        VertexFormat::F32x3 => vk::Format::R32G32B32_SFLOAT,
        VertexFormat::F32x4 => vk::Format::R32G32B32A32_SFLOAT,
        VertexFormat::U32 => vk::Format::R32_UINT,
        VertexFormat::U32x2 => vk::Format::R32G32_UINT,
        VertexFormat::U32x3 => vk::Format::R32G32B32_UINT,
        VertexFormat::U32x4 => vk::Format::R32G32B32A32_UINT,
    }
}

/// The view a sampler of GLSL type `ty` reads, and whether it compares
/// depth (`…Shadow`).
pub fn sampler_kind(ty: &str) -> (TextureKind, bool) {
    let shadow = ty.ends_with("Shadow");
    let kind = if ty.contains("CubeArray") || ty.contains("Cube") {
        TextureKind::CubeArray
    } else if ty.contains("2DArray") {
        TextureKind::D2Array
    } else if ty.contains("3D") {
        TextureKind::D3
    } else {
        TextureKind::D2
    };
    (kind, shadow)
}

pub fn view_type(kind: TextureKind) -> vk::ImageViewType {
    match kind {
        TextureKind::D2 => vk::ImageViewType::TYPE_2D,
        TextureKind::D2Array => vk::ImageViewType::TYPE_2D_ARRAY,
        TextureKind::D3 => vk::ImageViewType::TYPE_3D,
        TextureKind::CubeArray => vk::ImageViewType::CUBE_ARRAY,
    }
}

/// Array layers of the Vulkan image (a 3D texture's depth is not one).
pub fn array_layers(desc: &TextureDesc) -> u32 {
    match desc.kind {
        TextureKind::D3 => 1,
        _ => desc.layers.max(1),
    }
}

pub fn f32_to_f16(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xFF) as i32;
    let mantissa = bits & 0x7F_FFFF;
    if exponent == 0xFF {
        // Inf or NaN.
        return sign | 0x7C00 | if mantissa != 0 { 0x200 } else { 0 };
    }
    let e = exponent - 127 + 15;
    if e >= 0x1F {
        return sign | 0x7C00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        let m = (mantissa | 0x80_0000) >> (1 - e);
        // Round to nearest.
        let half = (m >> 13) + ((m >> 12) & 1);
        return sign | half as u16;
    }
    let half = ((e as u32) << 10) | (mantissa >> 13);
    // Round to nearest (may carry into the exponent, which is right).
    let half = half + ((mantissa >> 12) & 1);
    sign | half as u16
}

/// Texels of `upload` turned into the bytes of `format`, as GL converts
/// on upload. `None` when no conversion is needed (same layout).
pub fn convert_texels(data: &[u8], upload: UploadFormat, format: Format) -> Option<Vec<u8>> {
    let same = matches!(
        (upload, format),
        (UploadFormat::R8, Format::R8)
            | (UploadFormat::R16, Format::R16)
            | (UploadFormat::Rgba8, Format::Rgba8 | Format::Srgba8)
            | (UploadFormat::R32F, Format::R32F | Format::Depth32F | Format::Depth24)
    );
    if same {
        return None;
    }
    let n = upload.texel_size();
    let texels = data.len() / n;
    let decode = |t: &[u8]| -> [f32; 4] {
        let f = |i: usize| f32::from_le_bytes([t[i * 4], t[i * 4 + 1], t[i * 4 + 2], t[i * 4 + 3]]);
        match upload {
            UploadFormat::R8 => [t[0] as f32 / 255.0, 0.0, 0.0, 1.0],
            UploadFormat::R16 => [u16::from_le_bytes([t[0], t[1]]) as f32 / 65535.0, 0.0, 0.0, 1.0],
            UploadFormat::Rgba8 => [t[0] as f32 / 255.0, t[1] as f32 / 255.0, t[2] as f32 / 255.0, t[3] as f32 / 255.0],
            UploadFormat::R32F => [f(0), 0.0, 0.0, 1.0],
            UploadFormat::Rg32F => [f(0), f(1), 0.0, 1.0],
            UploadFormat::Rgba32F => [f(0), f(1), f(2), f(3)],
        }
    };
    let unorm8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    let mut out = Vec::with_capacity(texels * format.texel_size());
    for t in data.chunks_exact(n) {
        let v = decode(t);
        match format {
            Format::R8 => out.push(unorm8(v[0])),
            Format::R16 => out.extend_from_slice(&((v[0].clamp(0.0, 1.0) * 65535.0 + 0.5) as u16).to_le_bytes()),
            Format::R16F => out.extend_from_slice(&f32_to_f16(v[0]).to_le_bytes()),
            Format::R32F | Format::Depth32F | Format::Depth24 => out.extend_from_slice(&v[0].to_le_bytes()),
            Format::RG16F => {
                out.extend_from_slice(&f32_to_f16(v[0]).to_le_bytes());
                out.extend_from_slice(&f32_to_f16(v[1]).to_le_bytes());
            }
            Format::Rgba8 | Format::Srgba8 => out.extend(v.iter().map(|&c| unorm8(c))),
            Format::Rgba16F => {
                for c in v {
                    out.extend_from_slice(&f32_to_f16(c).to_le_bytes());
                }
            }
            Format::Rg11B10F => {
                let r = (f32_to_f16(v[0]) >> 4) as u32 & 0x7FF;
                let g = (f32_to_f16(v[1]) >> 4) as u32 & 0x7FF;
                let b = (f32_to_f16(v[2]) >> 5) as u32 & 0x3FF;
                out.extend_from_slice(&(r | (g << 11) | (b << 22)).to_le_bytes());
            }
            Format::Depth16 => out.extend_from_slice(&((v[0].clamp(0.0, 1.0) * 65535.0 + 0.5) as u16).to_le_bytes()),
            Format::Depth24Stencil8 => out.extend_from_slice(&v[0].to_le_bytes()),
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves() {
        assert_eq!(f32_to_f16(0.0), 0);
        assert_eq!(f32_to_f16(1.0), 0x3C00);
        assert_eq!(f32_to_f16(-2.0), 0xC000);
        assert_eq!(f32_to_f16(65504.0), 0x7BFF);
        assert_eq!(f32_to_f16(1e9), 0x7C00);
        assert_eq!(f32_to_f16(0.5), 0x3800);
    }

    #[test]
    fn sampler_types() {
        assert_eq!(sampler_kind("sampler2DArrayShadow"), (TextureKind::D2Array, true));
        assert_eq!(sampler_kind("samplerCubeArrayShadow"), (TextureKind::CubeArray, true));
        assert_eq!(sampler_kind("sampler3D"), (TextureKind::D3, false));
        assert_eq!(sampler_kind("sampler2D"), (TextureKind::D2, false));
    }
}
