//! The ground's colour seen from above, for reflection rays that meet it
//! (`groundAlbedo` in `rays.glsl`): the terrain drawn with its own program
//! from straight above into a texture, its colour instead of its light
//! (`baking_ground`: `shadeSurface` hands the albedo back). So reflections
//! show the real ground (roads, paths, soil, grass, wet and snowy as it is)
//! for any world, whatever its program does.
//!
//! Three levels around the camera: a near one, sharp, a far one over the
//! ground that is loaded, and a coarse one over the whole horizon (for
//! mirrors that look that far). Each is drawn again when the camera has
//! moved past its snapping step, and every so often for the weather and
//! time.

use std::cell::Cell;

use cgmath::{Matrix4, Point3, Vector3};

use crate::core::gfx::{self, Format, TextureDesc};

/// Texels per side of each level.
const SIZE: u32 = 2048;
/// Half the near level's side (metres).
const NEAR_RADIUS: f32 = 128.0;
/// Frames before a level is drawn again even where the camera stayed
/// (weather, time of day): the near one, the far one.
const NEAR_REFRESH: u32 = 90;
const FAR_REFRESH: u32 = 300;
const HORIZON_REFRESH: u32 = 900;
/// Half the horizon level's side: the horizon's whole reach (see
/// `terrain::horizon`), about 16 m a texel.
const HORIZON_RADIUS: f32 = (crate::terrain::horizon::FINEST_STEP
    * crate::terrain::horizon::HALF_CELLS as f64
    * (1 << (crate::terrain::horizon::LEVELS - 1)) as f64) as f32;
/// The horizon level moves in steps of this many metres.
const HORIZON_STEP: f32 = 1024.0;

thread_local! {
    static BAKING: Cell<bool> = const { Cell::new(false) };
}

/// Whether the ground is being drawn for its colour now: lit shaders hand
/// back their albedo (`albedoBake`), and the camera's view counts as drawn.
pub fn baking_ground() -> bool {
    BAKING.with(Cell::get)
}

struct Level {
    color: gfx::Texture,
    target: gfx::RenderTarget,
    /// World to the level's clip space, as drawn (`None`: never drawn).
    matrix: Option<Matrix4<f32>>,
    centre: Option<(i32, i32)>,
    age: u32,
}

impl Level {
    /// Drawn with `depth` (shared by the levels: one is drawn at a time).
    fn new(depth: &gfx::Texture) -> Self {
        let color = gfx::Texture::new(TextureDesc::d2(Format::Srgba8, SIZE, SIZE));
        let target = gfx::RenderTarget::new(&[color.attachment(0, None)], Some(depth.attachment(0, None)));
        Self { color, target, matrix: None, centre: None, age: 0 }
    }

    /// Draw the level again, around `eye` snapped to `step` metres, if it
    /// moved or aged; `draw` draws the ground with the matrix it gets.
    fn update(&mut self, eye: Point3<f32>, radius: f32, step: f32, refresh: u32, draw: &dyn Fn(&Matrix4<f32>)) -> bool {
        let centre = ((eye.x / step).round() as i32, (eye.z / step).round() as i32);
        self.age += 1;
        if self.centre == Some(centre) && self.age < refresh {
            return false;
        }
        self.centre = Some(centre);
        self.age = 0;
        let at = Point3::new(centre.0 as f32 * step, eye.y, centre.1 as f32 * step);
        let matrix = view(at, radius);
        gfx::begin_pass(self.target.reference(), (SIZE, SIZE), gfx::Load::Color([0.0; 4]), gfx::Load::Depth(1.0));
        BAKING.with(|b| b.set(true));
        draw(&matrix);
        BAKING.with(|b| b.set(false));
        gfx::end_pass();
        self.matrix = Some(matrix);
        true
    }
}

/// Straight down onto `at`, `radius` metres to each side, from far above
/// to far below.
fn view(at: Point3<f32>, radius: f32) -> Matrix4<f32> {
    let above = Point3::new(at.x, at.y + 5000.0, at.z);
    let look = Matrix4::look_at_rh(above, Point3::new(at.x, at.y, at.z), Vector3::unit_z());
    cgmath::ortho(-radius, radius, -radius, radius, 1.0, 10_000.0) * look
}

/// The three levels.
pub struct GroundMaps {
    near: Level,
    far: Level,
    horizon: Level,
    _depth: gfx::Texture,
}

/// Where the levels lie this frame, for the shaders.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroundMapPlaces {
    pub near: Matrix4<f32>,
    pub far: Matrix4<f32>,
    pub horizon: Matrix4<f32>,
}

impl GroundMaps {
    pub fn new() -> Self {
        let depth = gfx::Texture::new(TextureDesc::d2(Format::Depth32F, SIZE, SIZE));
        Self { near: Level::new(&depth), far: Level::new(&depth), horizon: Level::new(&depth), _depth: depth }
    }

    /// Draw what is due, around `eye`; the far level covers `radius`
    /// metres to each side. `draw` draws the ground (in a pass begun on
    /// the level) with the matrix it gets. True when anything was drawn
    /// (the caller sets its viewport again).
    pub fn update(&mut self, eye: Point3<f32>, radius: f32, draw: &dyn Fn(&Matrix4<f32>)) -> bool {
        // One level a frame: they share the depth buffer, and a level drawn
        // right after another in the same frame came out empty (the depth
        // not cleared between the passes, 2026-11-23). It spreads the cost
        // too; a level that is due waits a frame or two.
        self.near.update(eye, NEAR_RADIUS, NEAR_RADIUS / 8.0, NEAR_REFRESH, draw)
            || self.far.update(eye, radius, (radius / 8.0).max(32.0), FAR_REFRESH, draw)
            || self.horizon.update(eye, HORIZON_RADIUS, HORIZON_STEP, HORIZON_REFRESH, draw)
    }

    pub fn places(&self) -> Option<GroundMapPlaces> {
        Some(GroundMapPlaces { near: self.near.matrix?, far: self.far.matrix?, horizon: self.horizon.matrix? })
    }

    /// Raw ids of the levels (near, far, horizon), for debug views.
    pub fn textures(&self) -> [gfx::RawId; 3] {
        [self.near.color.raw(), self.far.color.raw(), self.horizon.color.raw()]
    }

    /// Bind the levels to `units` (near, far, horizon).
    pub fn bind(&self, units: [u32; 3]) {
        gfx::bind_texture(units[0], &self.near.color, &gfx::SamplerDesc::LINEAR_CLAMP);
        gfx::bind_texture(units[1], &self.far.color, &gfx::SamplerDesc::LINEAR_CLAMP);
        gfx::bind_texture(units[2], &self.horizon.color, &gfx::SamplerDesc::LINEAR_CLAMP);
    }
}
