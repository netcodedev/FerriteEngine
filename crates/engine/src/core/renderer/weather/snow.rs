//! Trails pressed into the snow around the camera: a map of how far the
//! snow is pressed down, where feet pushed it up around their prints, kept
//! from frame to frame (moved along with the camera) and slowly filled in
//! again while it snows.

use std::time::Instant;

use cgmath::{InnerSpace, Point3};

use super::{Footprint, MAX_FOOTPRINTS, SNOW_TRAIL_UNIT};
use crate::core::{
    gfx::{self, Format, Load, SamplerDesc, TextureDesc, TextureKind},
    renderer::shader::Shader,
};

/// Texels per side.
const SIZE: i32 = 1024;
/// Mipmapped: the snow layer's coarser rings read the trails blurred to
/// their spacing (`snowDentLod`).
const SAMPLER: SamplerDesc = SamplerDesc::LINEAR_CLAMP.mipmapped();
/// Metres the map spans per side (about 5 cm per texel).
const EXTENT: f32 = 48.0;
/// The map moves in steps of this many texels: a power of two, so the
/// texels of its mipmaps (which the snow layer reads, `snowDentBlurred`)
/// stay where they were in the world as it moves, up to that level, and
/// what is read from them does not jump.
const STEP: i32 = 32;
/// Seconds of full snowfall that fill a footprint again.
const REFILL_SECONDS: f32 = 90.0;

const VERTEX: &str = "#version 460 core
void main() {
    vec2 p = vec2((gl_VertexID << 1) & 2, gl_VertexID & 2);
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}";

struct Target {
    texture: gfx::Texture,
    target: gfx::RenderTarget,
}

impl Target {
    fn new() -> Self {
        let size = SIZE as u32;
        let texture = gfx::Texture::new(TextureDesc::d2(Format::RG16F, size, size).with_mips(0));
        let target = gfx::RenderTarget::new(&[texture.attachment(0, None)], None);
        gfx::begin_pass(target.reference(), (size, size), Load::Color([0.0; 4]), Load::Keep);
        gfx::end_pass();
        texture.generate_mips();
        Target { texture, target }
    }
}

pub struct SnowTrails {
    shader: Shader,
    targets: [Target; 2],
    /// Which target holds the current trails.
    current: usize,
    /// Texel the map's corner is at, in whole texels from the origin.
    origin: Option<[i32; 2]>,
    last_update: Instant,
}

impl SnowTrails {
    pub fn new() -> Self {
        let shader = Shader::new(
            VERTEX,
            &format!("#version 460 core\n{}", include_str!("snow_update.glsl")),
        );
        let targets = [Target::new(), Target::new()];
        Self {
            shader,
            targets,
            current: 0,
            origin: None,
            last_update: Instant::now(),
        }
    }

    /// Move the map along with the camera at `eye`, fill the trails in by
    /// what fell since the last update (`snowfall` 0..1) and press in
    /// `footprints`. The pass that was running goes on after, with the
    /// viewport over all of its target.
    pub fn update(&mut self, eye: Point3<f32>, snowfall: f32, footprints: &[Footprint]) {
        let texel = EXTENT / SIZE as f32;
        let corner = |v: f32| {
            (((v - EXTENT * 0.5) / texel).floor() as i32).div_euclid(STEP) * STEP
        };
        let origin = [corner(eye.x), corner(eye.z)];
        // Nothing carries over from a map far away (or none at all).
        let shift = match self.origin {
            Some(previous) => [origin[0] - previous[0], origin[1] - previous[1]],
            None => [SIZE * 4, SIZE * 4],
        };
        self.origin = Some(origin);
        let dt = self.last_update.elapsed().as_secs_f32().min(1.0);
        self.last_update = Instant::now();

        let shader = &self.shader;
        // A triangle over the whole map (from `gl_VertexID`), always
        // filled: it computes the map.
        let desc = gfx::PipelineDesc {
            depth: gfx::DepthState::OFF,
            polygon: gfx::PolygonMode::Solid,
            ..Default::default()
        };
        // The shader takes `MAX_FOOTPRINTS` at a time: one step per batch,
        // the first also moving the map and filling it in.
        let batches: Vec<&[Footprint]> = if footprints.is_empty() {
            vec![&[]]
        } else {
            footprints.chunks(MAX_FOOTPRINTS).collect()
        };
        for (n, footprints) in batches.into_iter().enumerate() {
            let (from, to) = (self.current, 1 - self.current);
            let (shift, refill) = if n == 0 {
                (shift, snowfall * dt / REFILL_SECONDS)
            } else {
                ([0, 0], 0.0)
            };
            let size = SIZE as u32;
            gfx::begin_pass(self.targets[to].target.reference(), (size, size), Load::Keep, Load::Keep);
            gfx::set_pipeline(shader.pipeline(&desc));
            shader.set_uniform_1i("previous", 0);
            shader.set_uniform_2f("origin", origin[0] as f32 * texel, origin[1] as f32 * texel);
            shader.set_uniform_1f("texel", texel);
            shader.set_uniform_2i("shift", shift[0], shift[1]);
            shader.set_uniform_1f("refill", refill);
            shader.set_uniform_1i("stampCount", footprints.len() as i32);
            for (i, footprint) in footprints.iter().enumerate() {
                let forward = if footprint.forward.magnitude2() > 1e-8 {
                    footprint.forward.normalize()
                } else {
                    cgmath::Vector2::new(0.0, 1.0)
                };
                shader.set_uniform_4f(
                    &format!("stamps[{i}]"),
                    footprint.position.x,
                    footprint.position.z,
                    forward.x,
                    forward.y,
                );
                shader.set_uniform_1f(&format!("stampLength[{i}]"), footprint.length);
            }
            gfx::bind_texture(0, &self.targets[from].texture, &SAMPLER);
            gfx::draw(0, 3);
            gfx::end_pass();
            self.current = to;
        }
        gfx::unbind_texture(0, TextureKind::D2);
        self.targets[self.current].texture.generate_mips();
    }

    /// World x, z of the map's corner and the metres it spans.
    pub fn area(&self) -> Option<[f32; 3]> {
        let texel = EXTENT / SIZE as f32;
        self.origin
            .map(|[x, z]| [x as f32 * texel, z as f32 * texel, EXTENT])
    }

    pub fn bind(&self) {
        gfx::bind_texture(SNOW_TRAIL_UNIT, &self.targets[self.current].texture, &SAMPLER);
    }
}
