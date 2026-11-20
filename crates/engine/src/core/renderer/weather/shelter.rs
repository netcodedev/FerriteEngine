//! How open the sky is over the ground, smoothed: from the exposure map
//! (everything seen from above) and the ground map (the terrain alone), the
//! share of each spot that nothing well above the ground shelters, averaged
//! over the spot and blurred over about a metre. Snow under trees lies
//! thinner there instead of following every gap in the leaves; the snow
//! layer (`snow_layer.rs`) takes its height from this. It covers the same
//! area as the exposure map and is made whenever that is drawn.

use super::{exposure, exposure::SkyExposure, SHELTER_UNIT};
use crate::core::{
    gfx::{self, Format, Load, SamplerDesc, TextureDesc},
    renderer::shader::Shader,
};

/// Texels per side: four exposure map texels each (about 63 cm).
const SIZE: u32 = 256;
/// How the map is read.
const SAMPLER: SamplerDesc = SamplerDesc::LINEAR_CLAMP;

const VERTEX: &str = "#version 460 core
void main() {
    vec2 p = vec2((gl_VertexID << 1) & 2, gl_VertexID & 2);
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}";

/// The open share of each texel, from the exposure and ground maps.
const OPEN: &str = "#version 460 core
uniform sampler2D exposureMap;
uniform sampler2D groundMap;
uniform float exposureRange;
out float open;
void main() {
    ivec2 at = ivec2(gl_FragCoord.xy);
    ivec2 size = textureSize(exposureMap, 0);
    // Exposure map texels per texel of this one, on each side.
    int step = size.x / SHELTER_SIZE;
    float sheltered = 0.0;
    for (int j = 0; j < step; ++j) {
        for (int i = 0; i < step; ++i) {
            ivec2 t = at * step + ivec2(i, j);
            float ground = texelFetch(groundMap, t, 0).r;
            float top = texelFetch(exposureMap, t, 0).r;
            // How high the first thing from above is over the ground: only
            // what is well above it (a roof, a crown) shelters.
            float above = ground >= 1.0 ? 0.0 : (ground - top) * exposureRange;
            sheltered += smoothstep(0.5, 1.5, above);
        }
    }
    open = 1.0 - sheltered / float(step * step);
}";

/// One direction of a gaussian blur, `direction` (1, 0) or (0, 1).
const BLUR: &str = "#version 460 core
uniform sampler2D source;
uniform ivec2 direction;
out float result;
void main() {
    ivec2 at = ivec2(gl_FragCoord.xy);
    ivec2 last = textureSize(source, 0) - 1;
    // Sigma of about 1.6 texels (a metre).
    const float WEIGHTS[5] = float[](0.2501, 0.2047, 0.1123, 0.0412, 0.0102);
    float sum = texelFetch(source, at, 0).r * WEIGHTS[0];
    for (int i = 1; i < 5; ++i) {
        sum += texelFetch(source, clamp(at + direction * i, ivec2(0), last), 0).r * WEIGHTS[i];
        sum += texelFetch(source, clamp(at - direction * i, ivec2(0), last), 0).r * WEIGHTS[i];
    }
    result = sum;
}";

struct Target {
    texture: gfx::Texture,
    target: gfx::RenderTarget,
}

impl Target {
    fn new() -> Self {
        let texture = gfx::Texture::new(TextureDesc::d2(Format::R16F, SIZE, SIZE));
        let target = gfx::RenderTarget::new(&[texture.attachment(0, None)], None);
        // Open everywhere until the first update.
        gfx::begin_pass(target.reference(), (SIZE, SIZE), Load::Color([1.0; 4]), Load::Keep);
        gfx::end_pass();
        Target { texture, target }
    }

    fn begin(&self) {
        gfx::begin_pass(self.target.reference(), (SIZE, SIZE), Load::Keep, Load::Keep);
    }
}

pub struct ShelterMap {
    open: Shader,
    blur: Shader,
    /// The result is in the second; the first holds the half-blurred map.
    targets: [Target; 2],
}

/// A triangle over the whole target, made from `gl_VertexID`: no depth,
/// no blending, always filled (it computes a map).
fn fullscreen() -> gfx::PipelineDesc {
    gfx::PipelineDesc {
        depth: gfx::DepthState::OFF,
        polygon: gfx::PolygonMode::Solid,
        ..Default::default()
    }
}

impl ShelterMap {
    pub fn new() -> Self {
        let open = Shader::new(VERTEX, &OPEN.replace("SHELTER_SIZE", &SIZE.to_string()));
        let blur = Shader::new(VERTEX, BLUR);
        Self {
            open,
            blur,
            targets: [Target::new(), Target::new()],
        }
    }

    /// Make the map from `exposure`'s maps, just drawn. The pass that was
    /// running goes on after, with the viewport over all of its target;
    /// the caller restores the viewport.
    pub fn update(&self, exposure: &SkyExposure, range: f32) {
        let (sky, ground) = exposure.textures();
        let desc = fullscreen();

        // The open share, into the second target.
        self.targets[1].begin();
        gfx::set_pipeline(self.open.pipeline(&desc));
        self.open.set_uniform_1i("exposureMap", 0);
        self.open.set_uniform_1i("groundMap", 1);
        self.open.set_uniform_1f("exposureRange", range);
        gfx::bind_texture(0, sky, &exposure::SAMPLER);
        gfx::bind_texture(1, ground, &exposure::SAMPLER);
        gfx::draw(0, 3);
        gfx::unbind_texture(1, gfx::TextureKind::D2);
        gfx::end_pass();

        // Blurred across into the first, then down back into the second.
        for (from, to, direction) in [(1, 0, (1, 0)), (0, 1, (0, 1))] {
            self.targets[to].begin();
            gfx::set_pipeline(self.blur.pipeline(&desc));
            self.blur.set_uniform_1i("source", 0);
            self.blur.set_uniform_2i("direction", direction.0, direction.1);
            gfx::bind_texture(0, &self.targets[from].texture, &SAMPLER);
            gfx::draw(0, 3);
            gfx::end_pass();
        }
        gfx::unbind_texture(0, gfx::TextureKind::D2);
    }

    /// Bind the result (the second target) to `SHELTER_UNIT`.
    pub fn bind(&self) {
        gfx::bind_texture(SHELTER_UNIT, &self.targets[1].texture, &SAMPLER);
    }
}
