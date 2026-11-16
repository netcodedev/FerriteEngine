//! Where the ground is: the world's generator, apart from any drawing.
//!
//! Everything here is a pure function of the seed and a place in the
//! world, so a chunk comes out the same whichever thread makes it, in
//! whatever order, and however often it is unloaded and made again. The
//! generator owns no GL state and can run on loader threads (and, later, on
//! a server without a window).
//!
//! Positions are `f64` and chunk coordinates `i32` all the way down: `f32`
//! keeps millimetres only within a few kilometres of the origin.

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, OnceLock},
};

use super::CHUNK_SIZE;

pub mod biome;
pub mod caves;
pub mod hydrology;
pub mod map;
mod noise;
pub mod spline;
pub mod world;

pub use biome::{Biome, BiomeBlend};
pub use caves::{Underground3, CAVE_DEPTH};
use caves::Underground;
use hydrology::Hydrology;
pub use world::{Climate, WorldSettings, SEA_LEVEL};
use world::{smoothstep, Macro, WorldShape};

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

    /// The chunk position as the `Chunk` trait takes it (in chunks).
    pub fn as_position(&self) -> (f32, f32, f32) {
        (self.x as f32, self.y as f32, self.z as f32)
    }

    /// The coordinate of a chunk position as the `Chunk` trait takes it.
    pub fn from_position(position: (f32, f32, f32)) -> Self {
        Self::new(
            position.0.round() as i32,
            position.1.round() as i32,
            position.2.round() as i32,
        )
    }
}

/// Metres around a chunk's square its column holds heights for: enough for
/// the coarsest level of detail to sample a step beyond the chunk and for
/// the normals to look a metre further (see `DensityField`).
pub const COLUMN_MARGIN: i32 = 9;

/// The ground's height over one chunk's square, at whole metres, from
/// `COLUMN_MARGIN` before its corner to as far past its far side. Shared by
/// every chunk stacked over that square.
pub struct Column {
    /// Heights, `(x, z)` row by row from `-margin`.
    heights: Vec<f32>,
    /// Temperature at sea level (°C) and humidity on a coarser grid,
    /// `CLIMATE_STEP` metres apart from `CLIMATE_ORIGIN`, row by row.
    climate: Vec<[f32; 2]>,
    /// Lowest and highest height in the column (margin included).
    pub min_height: f32,
    pub max_height: f32,
}

/// The climate grid of a column: metres between its points, where it
/// starts (chunk-local) and its points per side, enough to cover the
/// column's margins.
const CLIMATE_STEP: i32 = 4;
const CLIMATE_ORIGIN: i32 = -12;
const CLIMATE_SPAN: i32 = (CHUNK_SIZE as i32 - 2 * CLIMATE_ORIGIN) / CLIMATE_STEP + 1;

impl Column {
    /// Samples per side.
    pub const SPAN: i32 = CHUNK_SIZE as i32 + 2 * COLUMN_MARGIN + 1;

    /// Height at chunk-local whole metres `(x, z)`, each in
    /// `-COLUMN_MARGIN ..= CHUNK_SIZE + COLUMN_MARGIN`.
    #[inline]
    pub fn height(&self, x: i32, z: i32) -> f32 {
        self.heights[((z + COLUMN_MARGIN) * Self::SPAN + (x + COLUMN_MARGIN)) as usize]
    }

    /// Temperature at sea level (°C) and humidity at chunk-local `(x, z)`
    /// (anywhere within the margins), blended from the climate grid.
    pub fn climate(&self, x: f32, z: f32) -> [f32; 2] {
        let last = (CLIMATE_SPAN - 2) as f32;
        let gx = ((x - CLIMATE_ORIGIN as f32) / CLIMATE_STEP as f32).clamp(0.0, last + 1.0);
        let gz = ((z - CLIMATE_ORIGIN as f32) / CLIMATE_STEP as f32).clamp(0.0, last + 1.0);
        let (ix, iz) = (gx.floor().min(last), gz.floor().min(last));
        let (tx, tz) = (gx - ix, gz - iz);
        let at = |i: f32, j: f32| self.climate[(j as i32 * CLIMATE_SPAN + i as i32) as usize];
        let (a, b, c, d) = (at(ix, iz), at(ix + 1.0, iz), at(ix, iz + 1.0), at(ix + 1.0, iz + 1.0));
        std::array::from_fn(|k| {
            let top = a[k] + (b[k] - a[k]) * tx;
            let bottom = c[k] + (d[k] - c[k]) * tx;
            top + (bottom - top) * tz
        })
    }

    /// The biomes at chunk-local `(x, z)` on ground of height `h`.
    pub fn biome(&self, x: f32, z: f32, h: f32) -> BiomeBlend {
        let [t, humidity] = self.climate(x, z);
        biome::biome_at(t, humidity, h)
    }
}

/// How many columns the generator keeps. Stacked chunks and chunks made
/// again at another level of detail find theirs here.
const COLUMN_CACHE: usize = 512;

/// Metres between the points of the large shape's grid (see `MacroTile`).
pub const MACRO_STEP: f64 = 4.0;
/// Grid points per tile side, without the border.
const TILE_POINTS: i32 = CHUNK_SIZE as i32 / MACRO_STEP as i32;
/// How many tiles of the large shape the generator keeps.
const TILE_CACHE: usize = 2048;

/// The large shape (see `world`) over one chunk square, on the 4 m grid,
/// with one point more before and two more after each side: what smooth
/// (bicubic) interpolation needs anywhere in the square.
struct MacroTile {
    /// `(i, z)` row by row, from `-1` to `TILE_POINTS + 1`.
    points: Vec<Macro>,
}

impl MacroTile {
    const SPAN: i32 = TILE_POINTS + 3;

    fn at(&self, i: i32, j: i32) -> Macro {
        self.points[((j + 1) * Self::SPAN + (i + 1)) as usize]
    }
}

/// A small least-recently-made cache.
struct Cache<V> {
    entries: HashMap<(i32, i32), Arc<V>>,
    /// Oldest first, for eviction.
    order: VecDeque<(i32, i32)>,
    capacity: usize,
}

impl<V> Cache<V> {
    fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            capacity,
        }
    }

    fn get(&self, key: (i32, i32)) -> Option<Arc<V>> {
        self.entries.get(&key).cloned()
    }

    /// Keep `value` (unless another thread was faster) and return what is
    /// kept.
    fn insert(&mut self, key: (i32, i32), value: Arc<V>) -> Arc<V> {
        if let Some(existing) = self.entries.get(&key) {
            return existing.clone();
        }
        self.entries.insert(key, value.clone());
        self.order.push_back(key);
        while self.order.len() > self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
        value
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

/// The world's generator: shared (in an `Arc`) by the terrain, its loader
/// threads and its chunks.
pub struct WorldGenerator {
    seed: u64,
    shape: WorldShape,
    underground: Underground,
    tiles: Mutex<Cache<MacroTile>>,
    columns: Mutex<Cache<Column>>,
    /// Rivers and lakes, planned when first needed (see `hydrology`);
    /// `None` inside when the world has none.
    hydrology: OnceLock<Option<Arc<Hydrology>>>,
}

/// Catmull-Rom weights for the four points around `t` (0..1 between the
/// second and third).
fn cubic_weights(t: f64) -> [f64; 4] {
    let (t2, t3) = (t * t, t * t * t);
    [
        0.5 * (-t3 + 2.0 * t2 - t),
        0.5 * (3.0 * t3 - 5.0 * t2 + 2.0),
        0.5 * (-3.0 * t3 + 4.0 * t2 + t),
        0.5 * (t3 - t2),
    ]
}

impl WorldGenerator {
    pub fn new(seed: u64) -> Arc<Self> {
        Self::with_settings(seed, WorldSettings::default())
    }

    pub fn with_settings(seed: u64, settings: WorldSettings) -> Arc<Self> {
        Arc::new(Self {
            seed,
            underground: Underground::new(seed, &settings),
            shape: WorldShape::new(seed, settings),
            tiles: Mutex::new(Cache::new(TILE_CACHE)),
            columns: Mutex::new(Cache::new(COLUMN_CACHE)),
            hydrology: OnceLock::new(),
        })
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }

    pub fn settings(&self) -> &WorldSettings {
        &self.shape.settings
    }

    /// The climate maps at world `(x, z)`, straight from the noise.
    pub fn climate(&self, x: f64, z: f64) -> Climate {
        self.shape.climate(x, z)
    }

    /// The large shape at `(x, z)`, without the hills, straight from the
    /// noise (not interpolated): for maps and far views.
    pub fn coarse_height(&self, x: f64, z: f64) -> f32 {
        self.shape.macro_at(x, z).height
    }

    /// The plan of the world's rivers and lakes, if it has them.
    pub fn hydrology(&self) -> Option<&Hydrology> {
        self.hydrology
            .get_or_init(|| self.settings().rivers.then(|| Hydrology::shared(self.seed, &self.shape)))
            .as_deref()
    }

    /// The river pieces that may reshape the ground of chunk square
    /// `(cx, cz)` (and its column's margins).
    fn rivers_near(&self, cx: i32, cz: i32) -> &[hydrology::Segment] {
        self.hydrology().map_or(&[], |h| h.segments_near(cx, cz))
    }

    /// The river pieces that may reshape the ground at world `(x, z)`.
    fn rivers_at(&self, x: f64, z: f64) -> &[hydrology::Segment] {
        let c = |v: f64| (v / CHUNK_SIZE as f64).floor() as i32;
        self.rivers_near(c(x), c(z))
    }

    /// The ground `h` at world `(x, z)` with the rivers' beds and the
    /// lakes' basins worked in; `rivers` the pieces near it.
    fn shaped(&self, rivers: &[hydrology::Segment], x: f64, z: f64, h: f32) -> f32 {
        let Some(plan) = self.hydrology() else {
            return h;
        };
        let lake = plan.lake_at(x, z);
        let h = lake.map_or(h, |(level, inside)| Hydrology::shape_lake(h, level, inside));
        let river = plan.nearest(rivers, x, z);
        let carved = Hydrology::carve(h, river);
        let Some((level, inside)) = lake else {
            return carved;
        };
        let keep = Hydrology::lake_keeps_floor(inside);
        if carved > h {
            // In a lake the river's banks do not raise its floor.
            return carved + (h - carved) * keep;
        }
        // A river leaving the lake lower than its water does not cut its
        // shore: the lake would spill down the river's bed. The river
        // starts below the shore instead.
        let outflow = river.is_some_and(|r| r.level < level - 0.3);
        if outflow {
            let shore = smoothstep(0.0, hydrology::LAKE_WATER as f64, inside as f64) as f32 * (1.0 - keep);
            return carved + (h - carved) * shore;
        }
        carved
    }

    /// Height of the water's surface at world `(x, z)`: the sea's, or a
    /// lake's or river's where one is. Where it lies under the ground
    /// there is no water.
    pub fn water_level(&self, x: f64, z: f64) -> f32 {
        let mut level = self.still_water_level(x, z);
        let river = self.hydrology().and_then(|plan| plan.nearest(self.rivers_at(x, z), x, z));
        if let Some(river) = river {
            if river.distance < river.width * 0.5 + 2.0 {
                level = level.max(river.level);
            }
        }
        level
    }

    /// The level of the sea or a lake at world `(x, z)`, without the
    /// rivers (too narrow for the horizon's coarse grid).
    pub fn still_water_level(&self, x: f64, z: f64) -> f32 {
        let lake = self
            .hydrology()
            .and_then(|plan| plan.lake_at(x, z))
            .filter(|&(_, inside)| inside >= hydrology::LAKE_WATER);
        lake.map_or(SEA_LEVEL, |(level, _)| level.max(SEA_LEVEL))
    }

    fn tile(&self, tx: i32, tz: i32) -> Arc<MacroTile> {
        if let Some(tile) = self.tiles.lock().unwrap().get((tx, tz)) {
            return tile;
        }
        // Made outside the lock: two threads may both make it, the same.
        let (x0, z0) = (tx as f64 * CHUNK_SIZE as f64, tz as f64 * CHUNK_SIZE as f64);
        let span = MacroTile::SPAN;
        let mut points = Vec::with_capacity((span * span) as usize);
        for j in -1..span - 1 {
            for i in -1..span - 1 {
                points.push(
                    self.shape
                        .macro_at(x0 + i as f64 * MACRO_STEP, z0 + j as f64 * MACRO_STEP),
                );
            }
        }
        let tile = Arc::new(MacroTile { points });
        self.tiles.lock().unwrap().insert((tx, tz), tile)
    }

    /// The large shape at world `(x, z)`, interpolated from the 4 m grid.
    /// `tile` finds the tile of a chunk square.
    fn interpolated(&self, x: f64, z: f64, tile: &mut impl FnMut(i32, i32) -> Arc<MacroTile>) -> Macro {
        let (gx, gz) = (x / MACRO_STEP, z / MACRO_STEP);
        let (fx, fz) = (gx.floor(), gz.floor());
        let (ix, iz) = (fx as i64, fz as i64);
        let (tx, tz) = (ix.div_euclid(TILE_POINTS as i64), iz.div_euclid(TILE_POINTS as i64));
        let (i, j) = (
            ix.rem_euclid(TILE_POINTS as i64) as i32,
            iz.rem_euclid(TILE_POINTS as i64) as i32,
        );
        let tile = tile(tx as i32, tz as i32);
        let (wx, wz) = (cubic_weights(gx - fx), cubic_weights(gz - fz));
        let (mut height, mut hills, mut temperature, mut humidity) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for (b, wz) in wz.iter().enumerate() {
            for (a, wx) in wx.iter().enumerate() {
                let m = tile.at(i + a as i32 - 1, j + b as i32 - 1);
                let w = wx * wz;
                height += w * m.height as f64;
                hills += w * m.hills as f64;
                temperature += w * m.temperature as f64;
                humidity += w * m.humidity as f64;
            }
        }
        Macro {
            height: height as f32,
            hills: hills.max(0.0) as f32,
            temperature: temperature as f32,
            humidity: humidity.clamp(0.0, 1.0) as f32,
        }
    }

    /// Height of the ground at world `(x, z)`, in metres: where the
    /// density field crosses zero over that point.
    pub fn surface_height(&self, x: f64, z: f64) -> f32 {
        let m = self.interpolated(x, z, &mut |tx, tz| self.tile(tx, tz));
        self.shaped(self.rivers_at(x, z), x, z, m.height + m.hills * self.shape.detail(x, z))
    }

    /// The biomes at world `(x, z)`, on the ground there.
    pub fn surface_biome(&self, x: f64, z: f64) -> BiomeBlend {
        let m = self.interpolated(x, z, &mut |tx, tz| self.tile(tx, tz));
        let h = self.shaped(self.rivers_at(x, z), x, z, m.height + m.hills * self.shape.detail(x, z));
        biome::biome_at(m.temperature, m.humidity, h)
    }

    /// How steep the ground is at world `(x, z)`: rise per metre. The
    /// ground as the hills shape it, before rivers and lakes are worked in:
    /// their banks are steep, but should not overhang or open into caves.
    fn slope(&self, x: f64, z: f64) -> f32 {
        let natural = |x: f64, z: f64| {
            let m = self.interpolated(x, z, &mut |tx, tz| self.tile(tx, tz));
            m.height + m.hills * self.shape.detail(x, z)
        };
        let dx = natural(x + 1.0, z) - natural(x - 1.0, z);
        let dz = natural(x, z + 1.0) - natural(x, z - 1.0);
        0.5 * dx.hypot(dz)
    }

    /// Whether the ground at world `(x, z)` is the plain height field
    /// there: no overhang pushes it and no cave opens under it. What
    /// stands on `surface_height` stands on the ground there.
    pub fn is_plain_ground(&self, x: f64, z: f64) -> bool {
        let h = self.surface_height(x, z);
        if self.underground.has_overhangs() && self.underground.overhang_amplitude(self.slope(x, z)) > 0.25 {
            return false;
        }
        self.underground.cave(x, h as f64 - 1.5, z, h, self.slope(x, z)) < -1.0
    }

    /// The ground as a volume at world `(x, y, z)`: negative inside,
    /// positive in the air (roughly metres from the surface), with caves
    /// and overhangs. Straight from the noise: chunks blend the same
    /// values from a 4 m lattice, so they differ from this a little.
    pub fn density(&self, x: f64, y: f64, z: f64) -> f32 {
        let h = self.surface_height(x, z);
        let u = &self.underground;
        let slope = self.slope(x, z);
        let push = if u.has_overhangs() { u.overhang(x, y, z, slope) } else { 0.0 };
        (y as f32 - h - push).max(u.cave(x, y, z, h, slope))
    }

    /// Heights between which caves may lie under ground whose surface
    /// lies between `low` and `high` (see `caves::Underground::cave_band`).
    pub fn cave_band(&self, low: f32, high: f32) -> Option<(f32, f32)> {
        self.underground.cave_band(low, high)
    }

    /// Caves and overhangs around the chunk at `coord` (see
    /// `Underground3`), or `None` where it has neither.
    /// `None` too unless `detail`: from afar neither shows, and both cost.
    pub fn underground(&self, coord: ChunkCoord, detail: bool) -> Option<Underground3> {
        let u = &self.underground;
        if !detail || (!u.has_caves() && !u.has_overhangs()) {
            return None;
        }
        let column = self.column(coord.x, coord.z);
        let floor = coord.y as f32 * CHUNK_SIZE as f32;
        let ceiling = floor + CHUNK_SIZE as f32;
        // Steps reach 8 m and the lattice 12 m beyond the chunk.
        let (low, high) = (floor - 12.0, ceiling + 12.0);
        let caves = u
            .cave_band(column.min_height, column.max_height)
            .is_some_and(|(bottom, top)| bottom <= high && top + 12.0 >= low);
        let reach = caves::OVERHANG_DEPTH * self.settings().overhangs.max(0.0);
        let overhangs = u.has_overhangs()
            && column.max_height - column.min_height > 40.0
            && column.min_height - reach <= high
            && column.max_height + reach >= low;
        if !caves && !overhangs {
            return None;
        }
        let [x0, y0, z0] = coord.origin();
        let span = Underground3::SPAN;
        let at = |i: i32| (Underground3::ORIGIN + i * Underground3::STEP) as f64;
        // The surface and its slope over the lattice's columns.
        let mut ground = Vec::with_capacity((span * span) as usize);
        for k in 0..span {
            for i in 0..span {
                let (x, z) = (x0 + at(i), z0 + at(k));
                let h = self.surface_height(x, z);
                let slope = self.slope(x, z);
                ground.push((h, slope));
            }
        }
        let mut values = Vec::with_capacity((span * span * span) as usize);
        let mut any = false;
        for k in 0..span {
            for j in 0..span {
                for i in 0..span {
                    let (x, y, z) = (x0 + at(i), y0 + at(j), z0 + at(k));
                    let (h, slope) = ground[(k * span + i) as usize];
                    let near_surface = (y as f32 - h).abs() < caves::OVERHANG_DEPTH + 8.0;
                    let push = if overhangs && near_surface { u.overhang(x, y, z, slope) } else { 0.0 };
                    let cave = if caves { u.cave_parts(x, y, z, h, slope) } else { caves::CaveParts::NONE };
                    any |= push != 0.0 || cave.value() > -12.0;
                    values.push(Underground3::entry(push, cave));
                }
            }
        }
        any.then(|| Underground3::new(values))
    }

    /// The ground at world `(x, z)` on the 4 m grid of the large shape
    /// (both multiples of `MACRO_STEP`): its height, exactly as
    /// `surface_height` gives it there, its biomes and the level of the
    /// sea or a lake there (see `still_water_level`). Needs no cached tiles, so it suits
    /// samples spread far apart (the horizon).
    pub fn grid_sample(&self, x: f64, z: f64) -> (f32, BiomeBlend, f32) {
        let m = self.shape.macro_at(x, z);
        let h = self.shaped(self.rivers_at(x, z), x, z, m.height + m.hills * self.shape.detail(x, z));
        (h, biome::biome_at(m.temperature, m.humidity, h), self.still_water_level(x, z))
    }

    /// The column of chunk square `(cx, cz)`, from the cache or made now.
    pub fn column(&self, cx: i32, cz: i32) -> Arc<Column> {
        if let Some(column) = self.columns.lock().unwrap().get((cx, cz)) {
            return column;
        }
        // Made outside the lock: two threads may both make it, the same.
        let column = Arc::new(self.make_column(cx, cz));
        self.columns.lock().unwrap().insert((cx, cz), column)
    }

    fn make_column(&self, cx: i32, cz: i32) -> Column {
        let x0 = cx as f64 * CHUNK_SIZE as f64;
        let z0 = cz as f64 * CHUNK_SIZE as f64;
        // The tiles around this square, fetched once.
        let mut near: HashMap<(i32, i32), Arc<MacroTile>> = HashMap::new();
        let mut tile = |tx: i32, tz: i32| near.entry((tx, tz)).or_insert_with(|| self.tile(tx, tz)).clone();
        let span = Column::SPAN;
        let rivers = self.rivers_near(cx, cz);
        let mut heights = Vec::with_capacity((span * span) as usize);
        let (mut min_height, mut max_height) = (f32::MAX, f32::MIN);
        for z in -COLUMN_MARGIN..span - COLUMN_MARGIN {
            for x in -COLUMN_MARGIN..span - COLUMN_MARGIN {
                let (wx, wz) = (x0 + x as f64, z0 + z as f64);
                let m = self.interpolated(wx, wz, &mut tile);
                let h = self.shaped(rivers, wx, wz, m.height + m.hills * self.shape.detail(wx, wz));
                min_height = min_height.min(h);
                max_height = max_height.max(h);
                heights.push(h);
            }
        }
        let mut climate = Vec::with_capacity((CLIMATE_SPAN * CLIMATE_SPAN) as usize);
        for j in 0..CLIMATE_SPAN {
            for i in 0..CLIMATE_SPAN {
                let wx = x0 + (CLIMATE_ORIGIN + i * CLIMATE_STEP) as f64;
                let wz = z0 + (CLIMATE_ORIGIN + j * CLIMATE_STEP) as f64;
                let m = self.interpolated(wx, wz, &mut tile);
                climate.push([m.temperature, m.humidity]);
            }
        }
        Column {
            heights,
            climate,
            min_height,
            max_height,
        }
    }
}

#[cfg(test)]
mod tests;
