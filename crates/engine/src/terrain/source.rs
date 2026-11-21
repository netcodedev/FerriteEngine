//! What the engine asks of a world: the contract between the terrain
//! (streaming, meshing, the horizon, water, scattered models, drawing) and
//! whatever decides where the ground is and what it looks like.
//!
//! A game brings its own world by implementing `TerrainSource` (and
//! `TerrainGenerator`, so scenes can name it) in its scripts; the engine
//! comes with one of its own, `Hills` (see `hills`), for projects that have
//! none. The engine knows nothing of biomes, rivers or roads: a source
//! hands it a density field per chunk, a colour and sixteen free numbers per
//! surface point (`SurfaceSample`) and the shaders that make sense of them.
//!
//! Everything a source answers must be a pure function of its seed,
//! settings and the place asked about: chunks are made on loader threads,
//! in any order, and made again whenever they come back into view.

use std::{any::Any, ops::RangeInclusive, sync::Arc};

use cgmath::Point3;
use image::RgbaImage;
use serde::{de::DeserializeOwned, Serialize};

use crate::core::{
    entity::{
        component::{Property, PropertyValue},
        Entity,
    },
    renderer::{light::light::Light, shader::Shader, texture::Texture},
    scene::Scene,
};

use super::{shaders, CHUNK_SIZE};

/// Metres a chunk's field reaches past its square on every side: enough for
/// the coarsest level of detail to sample a step beyond the chunk and for
/// the normals to look a metre further.
pub const FIELD_MARGIN: i32 = 9;
/// Samples per side of a chunk's field (whole metres, from `-FIELD_MARGIN`).
pub const FIELD_SPAN: i32 = CHUNK_SIZE as i32 + 2 * FIELD_MARGIN + 1;

/// A chunk's place in the grid of chunks: chunk `(x, y, z)` covers world
/// metres `x * CHUNK_SIZE .. (x + 1) * CHUNK_SIZE`, and so on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChunkCoord {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl ChunkCoord {
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    /// The chunk that holds world point `(x, y, z)`.
    pub fn containing(x: f64, y: f64, z: f64) -> Self {
        let c = |v: f64| (v / CHUNK_SIZE as f64).floor() as i32;
        Self::new(c(x), c(y), c(z))
    }

    /// The chunk's corner with the smallest coordinates, in world metres.
    pub fn origin(&self) -> [f64; 3] {
        [self.x, self.y, self.z].map(|c| c as f64 * CHUNK_SIZE as f64)
    }

    /// The chunk's floor, in world metres.
    pub fn floor(&self) -> f32 {
        self.y as f32 * CHUNK_SIZE as f32
    }
}

/// The ground around one chunk as a density field: negative inside the
/// ground, positive in the air, roughly metres from the surface. Always a
/// height field (`height`), optionally refined by a volume (caves,
/// overhangs, tunnels: `fill_volume`) where the chunk has any.
///
/// Positions are chunk-local whole metres, each from `-FIELD_MARGIN` to
/// `CHUNK_SIZE + FIELD_MARGIN`.
#[derive(Clone)]
pub struct ChunkField {
    pub coord: ChunkCoord,
    /// Heights, `(x, z)` row by row from `-FIELD_MARGIN`.
    heights: Vec<f32>,
    /// The whole field, `(x, y, z)` with x fastest, then y, then z; `None`
    /// where the height field says it all.
    volume: Option<Vec<f32>>,
    /// The sharp part of `volume` at each of its points (see
    /// `fill_volume_layered`); `None` where it has none.
    sharp: Option<Vec<f32>>,
    /// The field as seen from afar (see `far_field`).
    far: Far,
    /// Lowest and highest height (margin included).
    pub min_height: f32,
    pub max_height: f32,
}

impl ChunkField {
    /// The field of the height field `heights` (`FIELD_SPAN` squared, as
    /// `height` reads them).
    pub fn from_heights(coord: ChunkCoord, heights: Vec<f32>) -> Self {
        assert_eq!(heights.len(), (FIELD_SPAN * FIELD_SPAN) as usize, "a chunk field's heights");
        let (min_height, max_height) = heights
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), &h| (lo.min(h), hi.max(h)));
        Self {
            coord,
            heights,
            volume: None,
            sharp: None,
            far: Far::HeightField,
            min_height,
            max_height,
        }
    }

    /// The field of the height field `height(world x, world z)`.
    pub fn from_fn(coord: ChunkCoord, mut height: impl FnMut(f64, f64) -> f32) -> Self {
        let [x0, _, z0] = coord.origin();
        let mut heights = Vec::with_capacity((FIELD_SPAN * FIELD_SPAN) as usize);
        for z in -FIELD_MARGIN..FIELD_SPAN - FIELD_MARGIN {
            for x in -FIELD_MARGIN..FIELD_SPAN - FIELD_MARGIN {
                heights.push(height(x0 + x as f64, z0 + z as f64));
            }
        }
        Self::from_heights(coord, heights)
    }

    /// The ground's height at chunk-local `(x, z)`, clamped to the field.
    #[inline]
    pub fn height(&self, x: i32, z: i32) -> f32 {
        let i = |v: i32| (v + FIELD_MARGIN).clamp(0, FIELD_SPAN - 1);
        self.heights[(i(z) * FIELD_SPAN + i(x)) as usize]
    }

    /// Make the field a volume: `density(x, y, z, ground)` at every
    /// chunk-local whole metre, where `ground` is what the height field
    /// alone says there (`y` above the height, negative under it).
    pub fn fill_volume(&mut self, mut density: impl FnMut(i32, i32, i32, f32) -> f32) {
        let floor = self.coord.floor();
        let mut volume = Vec::with_capacity((FIELD_SPAN * FIELD_SPAN * FIELD_SPAN) as usize);
        for z in -FIELD_MARGIN..FIELD_SPAN - FIELD_MARGIN {
            for y in -FIELD_MARGIN..FIELD_SPAN - FIELD_MARGIN {
                for x in -FIELD_MARGIN..FIELD_SPAN - FIELD_MARGIN {
                    let ground = floor + y as f32 - self.height(x, z);
                    volume.push(density(x, y, z, ground));
                }
            }
        }
        self.volume = Some(volume);
    }

    /// As `fill_volume`, with `density` giving each point's density in
    /// layers (see `VolumeSample`): its sharp part, which the source can
    /// also give between the whole metres (`ChunkSurface::sharp`), and the
    /// density as seen from afar.
    pub fn fill_volume_layered(&mut self, mut density: impl FnMut(i32, i32, i32, f32) -> VolumeSample) {
        let count = (FIELD_SPAN * FIELD_SPAN * FIELD_SPAN) as usize;
        let mut sharp = Vec::with_capacity(count);
        let mut far = Vec::with_capacity(count);
        let (mut any_sharp, mut far_differs) = (false, false);
        self.fill_volume(|x, y, z, ground| {
            let sample = density(x, y, z, ground);
            any_sharp |= sample.sharp != 0.0;
            far_differs |= sample.far != sample.value;
            sharp.push(sample.sharp);
            far.push(sample.far);
            sample.value
        });
        self.sharp = any_sharp.then_some(sharp);
        self.far = if far_differs { Far::Own(far) } else { Far::Volume };
    }

    /// The field as seen from afar, if it differs from the field: as the
    /// source gave it (`fill_volume_layered`), else the height field alone
    /// (a volume's caves, overhangs and tunnels, sampled as coarsely as from
    /// afar, would break through the slopes as holes). A source that makes
    /// its chunks without detail as it gives the far field of those with
    /// detail draws them alike from there.
    pub fn far_field(&self) -> Option<ChunkField> {
        self.volume.as_ref()?;
        let mut far = ChunkField::from_heights(self.coord, self.heights.clone());
        match &self.far {
            Far::HeightField => {}
            Far::Volume => return None,
            Far::Own(volume) => far.volume = Some(volume.clone()),
        }
        Some(far)
    }

    /// Whether the field has a sharp part anywhere.
    pub fn has_sharp(&self) -> bool {
        self.sharp.is_some()
    }

    /// The sharp part of the density at chunk-local `(x, y, z)` (clamped to
    /// the field), 0 where there is none.
    #[inline]
    pub fn sharp_at(&self, x: i32, y: i32, z: i32) -> f32 {
        match &self.sharp {
            None => 0.0,
            Some(sharp) => {
                let i = |v: i32| (v + FIELD_MARGIN).clamp(0, FIELD_SPAN - 1);
                sharp[((i(z) * FIELD_SPAN + i(y)) * FIELD_SPAN + i(x)) as usize]
            }
        }
    }

    /// Whether the field is more than its height field.
    pub fn has_volume(&self) -> bool {
        self.volume.is_some()
    }

    /// Density at chunk-local `(x, y, z)`, clamped to the field.
    #[inline]
    pub fn at(&self, x: i32, y: i32, z: i32) -> f32 {
        match &self.volume {
            None => self.coord.floor() + y as f32 - self.height(x, z),
            Some(volume) => {
                let i = |v: i32| (v + FIELD_MARGIN).clamp(0, FIELD_SPAN - 1);
                volume[((i(z) * FIELD_SPAN + i(y)) * FIELD_SPAN + i(x)) as usize]
            }
        }
    }

    /// Whether the chunk lies wholly in solid ground, with nothing to mesh.
    pub fn is_solid(&self) -> bool {
        self.volume.is_none() && self.coord.floor() + CHUNK_SIZE as f32 + 12.0 < self.min_height
    }
}

/// One point of a chunk's field in layers (see
/// `ChunkField::fill_volume_layered`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VolumeSample {
    /// The density there.
    pub value: f32,
    /// The part of it that is sharp detail (rock), given exactly between
    /// the samples by `ChunkSurface::sharp`: meshing finds the crossings and
    /// normals of the rest blended from the samples plus this exactly, so
    /// the edges stay sharp. 0 where there is none.
    pub sharp: f32,
    /// The density as seen from afar: what is left of the ground when what
    /// coarse sampling cannot draw (caves, tunnels) is left out.
    pub far: f32,
}

/// What a chunk's field looks like from afar.
#[derive(Clone)]
enum Far {
    /// The height field alone.
    HeightField,
    /// The volume as it is.
    Volume,
    /// Its own volume.
    Own(Vec<f32>),
}

/// Numbers per vertex a source may fill as it likes (`SurfaceSample`),
/// read by its shaders as `Data0`, `Data1`, `Data2` and `Data3` (vec4 each).
pub const SURFACE_DATA: usize = 16;

/// What a point of the ground looks like: a colour (display space) and
/// numbers only the source's shaders know the meaning of (which textures,
/// how wet, how cold...).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SurfaceSample {
    pub color: [f32; 3],
    pub data: [f32; SURFACE_DATA],
}

impl SurfaceSample {
    pub fn plain(color: [f32; 3]) -> Self {
        Self {
            color,
            data: [0.0; SURFACE_DATA],
        }
    }
}

/// A point of a chunk's mesh, for `ChunkSurface::surface`.
#[derive(Clone, Copy, Debug)]
pub struct SurfacePoint {
    /// Chunk-local position.
    pub local: [f32; 3],
    /// World position.
    pub world: [f64; 3],
    /// Unit normal, out of the ground.
    pub normal: [f32; 3],
    /// How far under the height field it lies (caves, under overhangs):
    /// positive under it.
    pub depth: f32,
}

/// One chunk's ground, as its source made it for meshing (see
/// `TerrainSource::chunk`). Lives on a loader thread while the chunk is
/// made: whatever the source looked up for the chunk can stay here.
pub trait ChunkSurface {
    fn field(&self) -> &ChunkField;
    /// What the ground looks like at a vertex of the mesh.
    fn surface(&self, point: &SurfacePoint) -> SurfaceSample;
    /// Whether `sharp` gives the sharp part between the samples (else
    /// meshing takes it from the samples like the rest). Default: when the
    /// field has one.
    fn exact_sharp(&self) -> bool {
        self.field().has_sharp()
    }
    /// The sharp part of the density at chunk-local `p`, anywhere (not only
    /// at whole metres), where the field has one (see
    /// `ChunkField::fill_volume_layered`): at whole metres exactly what
    /// the field holds. Default: none.
    fn sharp(&self, _p: [f32; 3]) -> f32 {
        0.0
    }
}

/// A chunk surface that is its field and one look everywhere.
pub struct PlainSurface<F: Fn(&SurfacePoint) -> SurfaceSample> {
    pub field: ChunkField,
    pub surface: F,
}

impl<F: Fn(&SurfacePoint) -> SurfaceSample> ChunkSurface for PlainSurface<F> {
    fn field(&self) -> &ChunkField {
        &self.field
    }

    fn surface(&self, point: &SurfacePoint) -> SurfaceSample {
        (self.surface)(point)
    }
}

/// The ground far away, for the horizon (see `horizon`).
#[derive(Clone, Copy, Debug)]
pub struct HorizonSample {
    /// The ground's height, as `surface_height` gives it.
    pub height: f32,
    pub surface: SurfaceSample,
    /// The level of still water there (the sea, a lake), or
    /// `f32::NEG_INFINITY`.
    pub water: f32,
}

/// A piece of river for the horizon (see `TerrainSource::far_rivers`): a
/// straight run from `a` to `b` (world x, z), with the water's level and
/// width at each end. Pieces that end where the next begins (the very same
/// point) are joined into one band.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RiverPiece {
    pub a: [f64; 2],
    pub b: [f64; 2],
    pub level: [f32; 2],
    pub width: [f32; 2],
}

/// A strip of ground for the horizon (see `TerrainSource::far_bands`): a
/// road, a path, anything too narrow for the horizon's grid. `points` run
/// along its middle (world x, z), each with how wide it is there (metres)
/// and what the ground looks like there.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceBand {
    pub points: Vec<([f64; 2], f32, SurfaceSample)>,
}

/// The programs the terrain draws a source's ground with. The vertex
/// program gets `position`, `normals`, `groundColor` and `data0..data3` at
/// locations 0 to 5 (see `shaders::STANDARD_VERTEX`); most sources keep the
/// standard one and write a fragment program starting with
/// `shaders::fragment_prelude()`.
#[derive(Clone, Debug)]
pub struct TerrainShaders {
    pub vertex: String,
    pub fragment: String,
    /// Depth alone, for depth-only passes; `None` draws them with the
    /// shading program.
    pub depth_vertex: Option<String>,
}

impl Default for TerrainShaders {
    /// The engine's: plain ground in its vertices' colours.
    fn default() -> Self {
        Self {
            vertex: shaders::STANDARD_VERTEX.to_string(),
            fragment: format!("{}\n{}", shaders::fragment_prelude(), shaders::DEFAULT_FRAGMENT),
            depth_vertex: Some(shaders::STANDARD_DEPTH_VERTEX.to_string()),
        }
    }
}

/// A world: where the ground is and what it looks like. See the module.
pub trait TerrainSource: Send + Sync + 'static {
    /// The chunks (by their `y`) of the column over chunk square `column`
    /// that may hold any ground or water, in full detail or not (see
    /// `chunk`); the others are not made. `vertical_range_for` helps.
    fn vertical_range(&self, column: (i32, i32), detail: bool) -> RangeInclusive<i32>;

    /// The ground of the chunk at `coord`. Without `detail` the chunk is
    /// only seen from afar: caves, overhangs and the like may be left out.
    fn chunk<'a>(&'a self, coord: ChunkCoord, detail: bool) -> Box<dyn ChunkSurface + 'a>;

    /// The ground's height at world `(x, z)`: where things stand.
    fn surface_height(&self, x: f64, z: f64) -> f32;

    /// The water's surface at world `(x, z)` (the sea's, a lake's, a
    /// river's); where it lies under the ground there is no water. Default:
    /// no water anywhere.
    fn water_level(&self, _x: f64, _z: f64) -> f32 {
        f32::NEG_INFINITY
    }

    /// How the water flows at world `(x, z)`, metres per second (x, z).
    fn water_flow(&self, _x: f64, _z: f64) -> [f32; 2] {
        [0.0; 2]
    }

    /// The sea's level, if the world has a sea: the far water's waves are
    /// the sea's there, and snow stays off the beach below it.
    fn sea_level(&self) -> Option<f32> {
        None
    }

    /// How likely a model of the scatter layer tagged `tag` (see
    /// `ScatterLayerData::tag`; empty if untagged) grows on the ground at
    /// world `(x, y, z)`, 0..1. Default: anywhere out of the water.
    fn scatter_chance(&self, _tag: &str, x: f64, y: f32, z: f64) -> f32 {
        if y > self.water_level(x, z) + 1.0 {
            1.0
        } else {
            0.0
        }
    }

    /// The colour scattered models standing at world `(x, z)` take on, as
    /// much as their output asks for (`SpawnMesh::tint`): e.g. the tint the
    /// ground's grass has there, so tufts turn dry or lush with it.
    /// Multiplies their linear colours. Default: white (none).
    fn scatter_tint(&self, _x: f64, _z: f64) -> [f32; 3] {
        [1.0; 3]
    }

    /// How likely a model of the scatter layer tagged `tag` grows at world
    /// `(x, z)` seen from far away, beyond the loaded chunks, where the
    /// horizon's ground stands at `height`. Asked for many spots at once
    /// (on a thread of its own), so a source whose `scatter_chance` is slow
    /// may answer from what it knows cheaply; it should mostly agree, or
    /// trees appear and vanish where the chunks take over. Default:
    /// `scatter_chance` at `height`.
    fn far_scatter_chance(&self, tag: &str, x: f64, z: f64, height: f32) -> f32 {
        self.scatter_chance(tag, x, height, z)
    }

    /// The rivers within `reach` metres of world `(x, z)` `centre`, for the
    /// horizon to draw beyond the loaded chunks (they are too narrow for
    /// its grid). Default: none.
    fn far_rivers(&self, _centre: [f64; 2], _reach: f64) -> Vec<RiverPiece> {
        Vec::new()
    }

    /// Strips of ground within `reach` metres of world `(x, z)` `centre`
    /// that the horizon should show although they are too narrow for its
    /// grid (roads, say): laid on its ground and drawn with the source's
    /// programs, as the ground around them. Default: none.
    fn far_bands(&self, _centre: [f64; 2], _reach: f64) -> Vec<SurfaceBand> {
        Vec::new()
    }

    /// The ground at world `(x, z)` as the horizon shows it.
    fn horizon_sample(&self, x: f64, z: f64) -> HorizonSample {
        HorizonSample {
            height: self.surface_height(x, z),
            surface: SurfaceSample::plain([0.3, 0.5, 0.2]),
            water: self.water_level(x, z),
        }
    }

    /// The programs the ground is drawn with (see `TerrainShaders`).
    fn shaders(&self) -> TerrainShaders {
        TerrainShaders::default()
    }

    /// Textures bound to units 0, 1, ... while the ground draws; may set
    /// uniforms of `shader` that stay fixed. Called on the GL thread.
    fn load_textures(&self, _shader: &Shader) -> Vec<Texture> {
        Vec::new()
    }

    /// What the world is still busy with before chunks are worth making
    /// (planning rivers, say), for a loading indicator; `None` when ready.
    /// Chunks are made either way.
    fn status(&self) -> Option<String> {
        None
    }

    /// The pictures of the whole world `map_image` can make (height,
    /// biomes...); none by default.
    fn map_layers(&self) -> Vec<String> {
        Vec::new()
    }

    /// A picture of the world from above, `size` pixels a side, of the
    /// layer named `layer`. Called on a thread of its own.
    fn map_image(&self, _size: u32, _layer: &str) -> Option<RgbaImage> {
        None
    }

    /// Lights of the world near `eye` (lamps), in world space.
    fn lights(&self, _eye: Point3<f32>) -> Vec<(Point3<f32>, Light)> {
        Vec::new()
    }

    /// Keep entities of the world's own (bridges, say) under the terrain's
    /// entity `terrain` in line with the camera at `eye`. Every frame, on
    /// the main thread.
    fn sync_scene(&self, _scene: &mut Scene, _terrain: &mut Entity, _eye: Point3<f32>) {}

    /// Take what `sync_scene` added out again: the world is replaced.
    fn clear_scene(&self, _scene: &mut Scene, _terrain: &mut Entity) {}

    /// The source as its own type (for scripts that ask their world more
    /// than this trait does).
    fn as_any(&self) -> &dyn Any;
}

/// The chunks of a column whose ground lies between `low` and `high`
/// metres: what a chunk's meshes reach past its floor and ceiling (a
/// coarse level's step and its skirt) included.
pub fn vertical_range_for(low: f32, high: f32) -> RangeInclusive<i32> {
    let reach = super::mesher::REACH;
    let chunk = |h: f32| (h / CHUNK_SIZE as f32).floor() as i32;
    chunk(low - reach)..=chunk(high + reach)
}

/// A world scenes can name: a `TerrainSource` with a name and settings.
/// Register it in the scripts with `ScriptRegistry::register_terrain`
/// (done for every `impl TerrainGenerator for ...` in a scripts folder's
/// top-level files).
pub trait TerrainGenerator: TerrainSource + Sized {
    /// Name it is saved under in scenes.
    const NAME: &'static str;
    /// Its settings' fields, in the order the inspector shows them.
    const SETTINGS: &'static [&'static str] = &[];
    type Settings: Serialize + DeserializeOwned + Default;

    /// The world of `seed` shaped by `settings`. May start work of its own
    /// (planning on a thread) and return at once.
    fn create(seed: u64, settings: Self::Settings) -> Arc<Self>;

    /// `settings` as the inspector shows them. Default: every field named
    /// in `SETTINGS`, as a script's parameters are shown.
    fn properties(settings: &Self::Settings) -> Vec<Property> {
        crate::core::script::parameters::properties(&crate::core::script::to_value(settings), Self::SETTINGS)
    }

    /// Set the setting shown as `name` (see `properties`); returns whether
    /// there was one.
    fn set_property(settings: &mut Self::Settings, name: &str, value: PropertyValue) -> bool {
        let mut data = crate::core::script::to_value(&*settings);
        if !crate::core::script::parameters::set(&mut data, name, value) {
            return false;
        }
        match data.into_rust::<Self::Settings>() {
            Ok(changed) => {
                *settings = changed;
                true
            }
            Err(e) => {
                log::warn!("world {}: cannot set {name}: {e}", Self::NAME);
                false
            }
        }
    }
}
