//! Pictures of a scatter model's variants from around it, for drawing them
//! far away as flat cards that turn toward the camera (see
//! `terrain::far_scatter`): each variant's most detailed tier, seen level
//! from `FRAMES` directions, its colour and its normals (model space), cut
//! out where it is not.

use cgmath::{Matrix4, Point3, Vector3};

use crate::core::{
    gfx::{self, Format, Load, SamplerDesc, TextureDesc, TextureKind},
    material::{Material, Pipeline},
};

use super::{vertex_layout, ModelVariant};

/// Directions a variant is pictured from, evenly around it: frame `k` from
/// model-space direction `(cos a, 0, sin a)`, `a = k * TAU / FRAMES`.
pub const FRAMES: usize = 8;
/// Pixels of a picture.
const WIDTH: u32 = 128;
const HEIGHT: u32 = 256;
/// How the pictures are read: blended between mip levels, clamped.
pub const SAMPLER: SamplerDesc = SamplerDesc::LINEAR_CLAMP.mipmapped();

/// One variant's pictures: two arrays of `FRAMES` layers each, the colour
/// (sRGB, alpha the cut-out, premultiplied) and the normal (`n * 0.5 + 0.5`,
/// premultiplied by the cut-out too).
pub struct Impostor {
    /// `None` where nothing is drawn (headless) or the model has no tier.
    pub albedo: Option<gfx::Texture>,
    pub normal: Option<gfx::Texture>,
    /// How far the pictures reach sideways from the model's upright axis,
    /// and from where to where they reach up (model space, metres).
    pub half_width: f32,
    pub bottom: f32,
    pub top: f32,
}

/// Picture `variant`. Must run on the render thread. Draws in a pass of
/// its own: the one running (if any) goes on after it as it was.
pub(super) fn bake(variant: &ModelVariant) -> Impostor {
    let (min, max) = variant.bounds;
    // However the model is turned, it stays within this of its axis.
    let half_width = (min[0].abs().max(max[0].abs())).hypot(min[2].abs().max(max[2].abs())).max(0.1);
    let (bottom, top) = (min[1], max[1].max(min[1] + 0.1));
    let mut impostor = Impostor {
        albedo: None,
        normal: None,
        half_width,
        bottom,
        top,
    };
    if !gfx::is_drawing() {
        return impostor;
    }
    let Some(lod) = variant.lods.first() else {
        return impostor;
    };
    let array = |format| {
        gfx::Texture::new(TextureDesc {
            kind: TextureKind::D2Array,
            layers: FRAMES as u32,
            mips: 0,
            ..TextureDesc::d2(format, WIDTH, HEIGHT)
        })
    };
    // The colour is written as linear light into the sRGB pictures.
    let (albedo, normal) = (array(Format::Srgba8), array(Format::Rgba8));
    let depth = gfx::Texture::new(TextureDesc::d2(Format::Depth24, WIDTH, HEIGHT));

    let outer = OuterPass::note();
    // The tier's submeshes, as the instances draw them (without the
    // instance attributes).
    let desc = gfx::PipelineDesc {
        vertex_layouts: crate::core::gfx::Layouts::of(&[vertex_layout()]),
        depth: gfx::DepthState::LESS,
        ..Default::default()
    };
    let middle = 0.5 * (bottom + top);
    let distance = 2.0 * half_width + 1.0;
    let projection = cgmath::ortho(
        -half_width,
        half_width,
        bottom - middle,
        top - middle,
        0.01,
        2.0 * distance,
    );
    for frame in 0..FRAMES {
        let layer = Some(frame as u32);
        let target = gfx::RenderTarget::new(
            &[albedo.attachment(0, layer), normal.attachment(0, layer)],
            Some(depth.attachment(0, None)),
        );
        gfx::begin_pass(target.reference(), (WIDTH, HEIGHT), Load::Color([0.0; 4]), Load::Depth(1.0));
        let angle = frame as f32 * std::f32::consts::TAU / FRAMES as f32;
        let eye = Point3::new(angle.cos() * distance, middle, angle.sin() * distance);
        let view = Matrix4::look_at_rh(eye, Point3::new(0.0, middle, 0.0), Vector3::unit_y());
        for sub in &lod.submeshes {
            let fallback;
            let material = match variant.model.material(sub.material_index) {
                Some(material) => material,
                None => {
                    fallback = Material::standard();
                    &fallback
                }
            };
            let shader = material.program(Pipeline::ImpostorBake);
            gfx::set_pipeline(shader.pipeline(&desc));
            shader.set_uniform_mat4("viewProjection", &(projection * view));
            shader.set_uniform_3f("cameraPosition", eye.x, eye.y, eye.z);
            material.bind(&shader);
            sub.bind();
            gfx::draw_indexed(0, sub.index_count, 0);
        }
        gfx::end_pass();
    }
    outer.restore();
    albedo.generate_mips();
    normal.generate_mips();
    impostor.albedo = Some(albedo);
    impostor.normal = Some(normal);
    impostor
}

/// What drew before the bake, to go on with after it: the pass (and its
/// viewport, which may be the editor's part of the window) and what it
/// changes about every pipeline, which the bake's own pass must not
/// inherit (the prepass's `LEQUAL`, an overlay's ignored depth).
struct OuterPass {
    viewport: gfx::Rect,
    overrides: gfx::PassOverrides,
}

impl OuterPass {
    fn note() -> Self {
        let overrides = gfx::pass_overrides();
        gfx::set_pass_overrides(gfx::PassOverrides::default());
        OuterPass { viewport: gfx::viewport(), overrides }
    }

    fn restore(self) {
        gfx::set_pass_overrides(self.overrides);
        gfx::set_viewport(self.viewport);
    }
}
