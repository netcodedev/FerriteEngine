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
    sync::{Arc, Mutex},
};

use libnoise::prelude::*;

use super::{CHUNK_SIZE, CHUNK_SIZE_FLOAT};

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
    /// Lowest and highest height in the column (margin included).
    pub min_height: f32,
    pub max_height: f32,
}

impl Column {
    /// Samples per side.
    pub const SPAN: i32 = CHUNK_SIZE as i32 + 2 * COLUMN_MARGIN + 1;

    /// Height at chunk-local whole metres `(x, z)`, each in
    /// `-COLUMN_MARGIN ..= CHUNK_SIZE + COLUMN_MARGIN`.
    #[inline]
    pub fn height(&self, x: i32, z: i32) -> f32 {
        self.heights[((z + COLUMN_MARGIN) * Self::SPAN + (x + COLUMN_MARGIN)) as usize]
    }
}

/// How many columns the generator keeps. Stacked chunks and chunks made
/// again at another level of detail find theirs here.
const COLUMN_CACHE: usize = 512;

/// The world's generator: shared (in an `Arc`) by the terrain, its loader
/// threads and its chunks.
pub struct WorldGenerator {
    seed: u64,
    /// The ground's height field, `-1..1`.
    height_noise: Fbm<2, Scale<2, Perlin<2>>>,
    columns: Mutex<ColumnCache>,
}

#[derive(Default)]
struct ColumnCache {
    columns: HashMap<(i32, i32), Arc<Column>>,
    /// Oldest first, for eviction.
    order: VecDeque<(i32, i32)>,
}

/// Added to every sample point, as the terrain always did: Perlin noise
/// repeats itself around the origin.
const NOISE_OFFSET: f64 = 16777216.0;

impl WorldGenerator {
    pub fn new(seed: u64) -> Arc<Self> {
        Arc::new(Self {
            seed,
            height_noise: Source::perlin(seed).scale([0.003; 2]).fbm(6, 1.0, 2.0, 0.5),
            columns: Mutex::new(ColumnCache::default()),
        })
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Height of the ground at world `(x, z)`, in metres: where the
    /// density field crosses zero over that point.
    pub fn surface_height(&self, x: f64, z: f64) -> f32 {
        let n = self.height_noise.sample([x + NOISE_OFFSET, z + NOISE_OFFSET]);
        ((1.0 + n) / 2.0) as f32 * CHUNK_SIZE_FLOAT - 1.0
    }

    /// The column of chunk square `(cx, cz)`, from the cache or made now.
    pub fn column(&self, cx: i32, cz: i32) -> Arc<Column> {
        if let Some(column) = self.columns.lock().unwrap().columns.get(&(cx, cz)) {
            return column.clone();
        }
        // Made outside the lock: two threads may both make it, the same.
        let column = Arc::new(self.make_column(cx, cz));
        let mut cache = self.columns.lock().unwrap();
        if !cache.columns.contains_key(&(cx, cz)) {
            cache.columns.insert((cx, cz), column.clone());
            cache.order.push_back((cx, cz));
            while cache.order.len() > COLUMN_CACHE {
                if let Some(oldest) = cache.order.pop_front() {
                    cache.columns.remove(&oldest);
                }
            }
        }
        column
    }

    fn make_column(&self, cx: i32, cz: i32) -> Column {
        let x0 = cx as f64 * CHUNK_SIZE as f64;
        let z0 = cz as f64 * CHUNK_SIZE as f64;
        let span = Column::SPAN;
        let mut heights = Vec::with_capacity((span * span) as usize);
        let (mut min_height, mut max_height) = (f32::MAX, f32::MIN);
        for z in -COLUMN_MARGIN..span - COLUMN_MARGIN {
            for x in -COLUMN_MARGIN..span - COLUMN_MARGIN {
                let h = self.surface_height(x0 + x as f64, z0 + z as f64);
                min_height = min_height.min(h);
                max_height = max_height.max(h);
                heights.push(h);
            }
        }
        Column {
            heights,
            min_height,
            max_height,
        }
    }
}

#[cfg(test)]
mod tests;
