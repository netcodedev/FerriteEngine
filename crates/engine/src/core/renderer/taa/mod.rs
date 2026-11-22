//! Temporal anti-aliasing and the view's motion vectors.
//!
//! With TAA on (graphics setting `display.taa`) the camera's view is drawn
//! without MSAA, its projection shifted by a different sub-pixel offset
//! every frame (`Taa::jitter`). After the scene is drawn, the motion
//! vectors say for every texel where its surface was a frame before
//! (`velocity.glsl` for the camera's own motion; moving models draw theirs
//! over it, see `motion`), and `resolve.glsl` blends the frame into the
//! history found there. Over a few frames every pixel collects samples
//! from all over its area: smooth edges, also on alpha-tested leaves and
//! grass, which MSAA does not smooth.
//!
//! The motion vectors are a texture of their own (`velocity`), kept for
//! what needs them later: the ray traced shadows' denoiser and DLSS.

pub mod motion;

use cgmath::{Matrix4, SquareMatrix};

use super::shader::Shader;
use crate::core::gfx::{self, Format, SamplerDesc, TextureDesc};

/// Format of the motion vectors: uv offsets (rg), the surface's distance
/// from the camera a frame before and now (b, a; clip w).
pub const VELOCITY_FORMAT: Format = Format::Rgba16F;
/// Format of the history: linear HDR light, as the scene, and each texel's
/// distance (clip w) in alpha.
const HISTORY_FORMAT: Format = Format::Rgba16F;
/// Share of the newest frame in the result while the history fits: about
/// the last ten frames make up a pixel.
const CURRENT_WEIGHT: f32 = 0.1;
/// Length of the jitter sequence.
const JITTER_SAMPLES: u32 = 8;

const CURRENT_UNIT: u32 = 0;
const HISTORY_UNIT: u32 = 1;
const VELOCITY_UNIT: u32 = 2;
const DEPTH_UNIT: u32 = 3;

/// Element `index` (from 1) of the Halton sequence in `base`, 0..1.
fn halton(mut index: u32, base: u32) -> f32 {
    let mut result = 0.0;
    let mut fraction = 1.0;
    while index > 0 {
        fraction /= base as f32;
        result += fraction * (index % base) as f32;
        index /= base;
    }
    result
}

/// The sub-pixel offset of frame `frame` for a view of `width` × `height`
/// texels, in NDC (a texel is 2 / width wide there): within half a texel
/// of the texel's centre.
pub fn jitter_offset(frame: u64, width: i32, height: i32) -> (f32, f32) {
    let index = (frame % JITTER_SAMPLES as u64) as u32 + 1;
    let x = halton(index, 2) - 0.5;
    let y = halton(index, 3) - 0.5;
    (x * 2.0 / width.max(1) as f32, y * 2.0 / height.max(1) as f32)
}

/// `view_projection` shifted by `offset` (NDC) on the screen.
pub fn jittered(view_projection: &Matrix4<f32>, offset: (f32, f32)) -> Matrix4<f32> {
    Matrix4::from_translation(cgmath::Vector3::new(offset.0, offset.1, 0.0)) * view_projection
}

/// The matrix `velocity.glsl` takes this frame's NDC to the last frame's
/// clip space with. Worked out in f64: the two matrices are nearly the
/// same, and their product is close to the identity, which f32 keeps
/// well, while the inverse alone (with far planes kilometres away) would
/// not.
pub fn reprojection(current: &Matrix4<f32>, previous: &Matrix4<f32>) -> Matrix4<f32> {
    let current: Matrix4<f64> = current.cast().expect("f32 fits f64");
    let previous: Matrix4<f64> = previous.cast().expect("f32 fits f64");
    match current.invert() {
        Some(inverse) => (previous * inverse).cast().expect("f64 fits f32"),
        None => Matrix4::identity(),
    }
}

/// Row w of `view_projection`'s inverse: its dot product with an NDC
/// position (w = 1) is 1 / clip w there. Depends on the projection only
/// (the view is rigid), so it keeps its precision far from the origin.
pub fn inverse_w(view_projection: &Matrix4<f32>) -> [f32; 4] {
    let m: Matrix4<f64> = view_projection.cast().expect("f32 fits f64");
    match m.invert() {
        // cgmath is column-major: row w is the w component of each column.
        Some(inverse) => [
            inverse.x.w as f32,
            inverse.y.w as f32,
            inverse.z.w as f32,
            inverse.w.w as f32,
        ],
        None => [0.0, 0.0, 0.0, 1.0],
    }
}

/// The textures TAA keeps, as big as the view.
pub struct TaaTargets {
    pub width: i32,
    pub height: i32,
    /// Motion vectors (`VELOCITY_FORMAT`).
    pub velocity: gfx::Texture,
    /// The camera's motion vectors are drawn into this, from the depth.
    velocity_target: gfx::RenderTarget,
    /// The same with the view's resolved depth, so moving models draw only
    /// where they are seen.
    motion_target: gfx::RenderTarget,
    history: [gfx::Texture; 2],
    history_targets: [gfx::RenderTarget; 2],
}

impl TaaTargets {
    /// Targets for a view of `width` × `height` whose resolved depth is
    /// `depth`.
    pub fn new(width: i32, height: i32, depth: &gfx::Texture) -> Self {
        let (w, h) = (width as u32, height as u32);
        let velocity = gfx::Texture::new(TextureDesc::d2(VELOCITY_FORMAT, w, h));
        let velocity_target = gfx::RenderTarget::new(&[velocity.attachment(0, None)], None);
        let motion_target = gfx::RenderTarget::new(&[velocity.attachment(0, None)], Some(depth.attachment(0, None)));
        let history = [
            gfx::Texture::new(TextureDesc::d2(HISTORY_FORMAT, w, h)),
            gfx::Texture::new(TextureDesc::d2(HISTORY_FORMAT, w, h)),
        ];
        let history_targets = [
            gfx::RenderTarget::new(&[history[0].attachment(0, None)], None),
            gfx::RenderTarget::new(&[history[1].attachment(0, None)], None),
        ];
        Self {
            width,
            height,
            velocity,
            velocity_target,
            motion_target,
            history,
            history_targets,
        }
    }
}

/// The temporal state: the jitter, the camera of the frame before, which
/// history is the newest.
pub struct Taa {
    velocity_shader: Shader,
    resolve_shader: Shader,
    frame: u64,
    /// The camera's view-projection this frame and the one before, both
    /// without jitter.
    view_projection: Option<Matrix4<f32>>,
    previous: Option<Matrix4<f32>>,
    /// This frame's view-projection with the jitter.
    jittered: Matrix4<f32>,
    /// The history last written.
    newest: usize,
    /// The newest history holds a frame of this view (not after a resize
    /// or a cut).
    history_valid: bool,
}

impl Taa {
    pub fn new() -> Self {
        let vertex = include_str!("../hdr/vertex.glsl");
        Self {
            velocity_shader: Shader::new(vertex, include_str!("velocity.glsl")),
            resolve_shader: Shader::new(vertex, include_str!("resolve.glsl")),
            frame: 0,
            view_projection: None,
            previous: None,
            jittered: Matrix4::identity(),
            newest: 0,
            history_valid: false,
        }
    }

    /// Forget the history (the targets were made anew, the camera cut).
    pub fn reset(&mut self) {
        self.history_valid = false;
        self.previous = None;
    }

    /// This frame's camera (without jitter); returns it with this frame's
    /// jitter for a view of `width` × `height`, which is what the scene is
    /// drawn with.
    pub fn begin_frame(&mut self, view_projection: Matrix4<f32>, width: i32, height: i32) -> Matrix4<f32> {
        self.frame = self.frame.wrapping_add(1);
        self.previous = self.view_projection.or(Some(view_projection));
        self.view_projection = Some(view_projection);
        self.jittered = jittered(&view_projection, jitter_offset(self.frame, width, height));
        self.jittered
    }

    /// The view-projections of this frame and the frame before (without
    /// jitter), once `begin_frame` was called.
    pub fn cameras(&self) -> Option<(Matrix4<f32>, Matrix4<f32>)> {
        Some((self.view_projection?, self.previous?))
    }

    /// The motion vectors into `targets.velocity`: the camera's, from
    /// `depth`, and over them the moving models' (`motion::draw`).
    pub fn draw_velocity(&self, targets: &TaaTargets, depth: &gfx::Texture, quad: &dyn Fn()) {
        let size = (targets.width as u32, targets.height as u32);
        gfx::begin_pass(
            targets.velocity_target.reference(),
            size,
            gfx::Load::Color([0.0; 4]),
            gfx::Load::Keep,
        );
        let shader = &self.velocity_shader;
        gfx::set_pipeline(shader.pipeline(&fullscreen()));
        gfx::bind_texture(DEPTH_UNIT, depth, &SamplerDesc::NEAREST_CLAMP);
        shader.set_uniform_1i("sceneDepth", DEPTH_UNIT as i32);
        let (reprojection, inverse_w) = match self.cameras() {
            Some((current, previous)) => (reprojection(&current, &previous), inverse_w(&current)),
            None => (Matrix4::identity(), [0.0, 0.0, 0.0, 1.0]),
        };
        shader.set_uniform_mat4("reprojection", &reprojection);
        let [x, y, z, w] = inverse_w;
        shader.set_uniform_4f("inverseW", x, y, z, w);
        quad();
        gfx::end_pass();
        gfx::unbind_texture(DEPTH_UNIT, gfx::TextureKind::D2);
        if let Some((current, previous)) = self.cameras() {
            if motion::pending() > 0 {
                gfx::begin_pass(
                    targets.motion_target.reference(),
                    size,
                    gfx::Load::Keep,
                    gfx::Load::Keep,
                );
                motion::draw(&self.jittered, &current, &previous);
                gfx::end_pass();
            }
        }
    }

    /// Blend `current` (this frame, resolved) into the history; returns
    /// the result, which stays the newest history until the next frame.
    pub fn resolve<'a>(
        &mut self,
        targets: &'a TaaTargets,
        current: &gfx::Texture,
        depth: &gfx::Texture,
        quad: &dyn Fn(),
    ) -> &'a gfx::Texture {
        let next = 1 - self.newest;
        gfx::begin_pass(
            targets.history_targets[next].reference(),
            (targets.width as u32, targets.height as u32),
            gfx::Load::Keep,
            gfx::Load::Keep,
        );
        let shader = &self.resolve_shader;
        gfx::set_pipeline(shader.pipeline(&fullscreen()));
        gfx::bind_texture(CURRENT_UNIT, current, &SamplerDesc::NEAREST_CLAMP);
        shader.set_uniform_1i("current", CURRENT_UNIT as i32);
        gfx::bind_texture(HISTORY_UNIT, &targets.history[self.newest], &SamplerDesc::LINEAR_CLAMP);
        shader.set_uniform_1i("history", HISTORY_UNIT as i32);
        gfx::bind_texture(VELOCITY_UNIT, &targets.velocity, &SamplerDesc::NEAREST_CLAMP);
        shader.set_uniform_1i("velocity", VELOCITY_UNIT as i32);
        gfx::bind_texture(DEPTH_UNIT, depth, &SamplerDesc::NEAREST_CLAMP);
        shader.set_uniform_1i("sceneDepth", DEPTH_UNIT as i32);
        shader.set_uniform_2f("texelSize", 1.0 / targets.width as f32, 1.0 / targets.height as f32);
        shader.set_uniform_1i("historyValid", self.history_valid as i32);
        shader.set_uniform_1f("currentWeight", CURRENT_WEIGHT);
        quad();
        gfx::end_pass();
        for unit in [CURRENT_UNIT, HISTORY_UNIT, VELOCITY_UNIT, DEPTH_UNIT] {
            gfx::unbind_texture(unit, gfx::TextureKind::D2);
        }
        self.newest = next;
        self.history_valid = true;
        &targets.history[next]
    }
}

impl Default for Taa {
    fn default() -> Self {
        Self::new()
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use cgmath::{Point3, Vector4};

    #[test]
    fn halton_starts_with_the_known_values() {
        assert_eq!(halton(1, 2), 0.5);
        assert_eq!(halton(2, 2), 0.25);
        assert_eq!(halton(3, 2), 0.75);
        assert!((halton(1, 3) - 1.0 / 3.0).abs() < 1e-6);
        assert!((halton(2, 3) - 2.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn jitter_stays_within_half_a_texel() {
        for frame in 0..32 {
            let (x, y) = jitter_offset(frame, 1920, 1080);
            assert!(x.abs() <= 1.0 / 1920.0 + 1e-7, "{x}");
            assert!(y.abs() <= 1.0 / 1080.0 + 1e-7, "{y}");
        }
    }

    #[test]
    fn jitter_repeats_after_the_sequence() {
        assert_eq!(
            jitter_offset(3, 800, 600),
            jitter_offset(3 + JITTER_SAMPLES as u64, 800, 600)
        );
        assert_ne!(jitter_offset(3, 800, 600), jitter_offset(4, 800, 600));
    }

    fn camera(eye: Point3<f32>) -> Matrix4<f32> {
        let projection = cgmath::perspective(cgmath::Deg(60.0), 16.0 / 9.0, 0.1, 20_000.0);
        projection
            * Matrix4::look_at_rh(
                eye,
                eye + cgmath::Vector3::new(0.0, 0.0, -1.0),
                cgmath::Vector3::unit_y(),
            )
    }

    #[test]
    fn a_still_camera_reprojects_onto_itself() {
        let vp = camera(Point3::new(5000.0, 120.0, -3000.0));
        let r = reprojection(&vp, &vp);
        let p = r * Vector4::new(0.3, -0.2, 0.97, 1.0);
        assert!((p.x / p.w - 0.3).abs() < 1e-4);
        assert!((p.y / p.w + 0.2).abs() < 1e-4);
    }

    #[test]
    fn a_point_lands_where_the_previous_camera_saw_it() {
        let previous = camera(Point3::new(5000.0, 120.0, -3000.0));
        let current = camera(Point3::new(5000.5, 120.0, -3000.0));
        let world = Vector4::new(5003.0, 121.0, -3020.0, 1.0);
        let now = current * world;
        let ndc = Vector4::new(now.x / now.w, now.y / now.w, now.z / now.w, 1.0);
        let before = previous * world;
        let p = reprojection(&current, &previous) * ndc;
        assert!((p.x / p.w - before.x / before.w).abs() < 1e-3);
        assert!((p.y / p.w - before.y / before.w).abs() < 1e-3);
        // The distances then and now, as `velocity.glsl` works them out.
        let w = inverse_w(&current);
        let now_w = 1.0 / (w[0] * ndc.x + w[1] * ndc.y + w[2] * ndc.z + w[3]);
        assert!((now_w - now.w).abs() < 1e-2 * now.w, "{now_w} {}", now.w);
        assert!((p.w * now_w - before.w).abs() < 1e-2 * before.w);
    }
}
