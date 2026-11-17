//! The engine's own world, for projects that bring none: rolling hills of
//! fractal noise, green on the slopes, sand at the water, rock where it is
//! steep (see `shaders::DEFAULT_FRAGMENT`), and optionally a sea.
//!
//! Also a small example of a `TerrainSource`; a game's world lives in its
//! scripts and is named in the scene (`Terrain(generator: "...")`).

use std::{any::Any, ops::RangeInclusive, sync::Arc};

use serde::{Deserialize, Serialize};

use super::{
    noise::Fractal,
    source::{
        vertical_range_for, ChunkCoord, ChunkSurface, ChunkField, HorizonSample, PlainSurface, SurfacePoint,
        SurfaceSample, TerrainGenerator, TerrainSource,
    },
    CHUNK_SIZE,
};

/// What shapes the hills.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct HillsSettings {
    /// Metres between the valleys and the hilltops, roughly.
    pub height: f32,
    /// Metres from one hill to the next, roughly.
    pub wavelength: f32,
    /// Whether the valleys fill with a sea (at height 0).
    pub sea: bool,
}

impl Default for HillsSettings {
    fn default() -> Self {
        Self {
            height: 60.0,
            wavelength: 900.0,
            sea: true,
        }
    }
}

pub struct Hills {
    settings: HillsSettings,
    shape: Fractal,
    detail: Fractal,
}

/// Height of the sea, where there is one.
const SEA_LEVEL: f32 = 0.0;

impl Hills {
    fn sea(&self) -> f32 {
        if self.settings.sea {
            SEA_LEVEL
        } else {
            f32::NEG_INFINITY
        }
    }

    fn height(&self, x: f64, z: f64) -> f32 {
        let h = self.settings.height;
        let shape = self.shape.at(x, z) as f32;
        let detail = self.detail.at(x, z) as f32;
        // Mostly above the sea, the deepest valleys under it.
        h * (0.35 + shape) + 0.12 * h * detail
    }

    /// The ground's colour at height `h` (display space).
    fn color(&self, h: f32) -> [f32; 3] {
        let grass = [0.30, 0.55, 0.16];
        let sand = [0.80, 0.74, 0.54];
        let shore = (h - self.sea() - 1.0).clamp(0.0, 2.0) / 2.0;
        std::array::from_fn(|k| sand[k] + (grass[k] - sand[k]) * shore)
    }
}

impl TerrainGenerator for Hills {
    const NAME: &'static str = "Hills";
    const SETTINGS: &'static [&'static str] = &["height", "wavelength", "sea"];
    type Settings = HillsSettings;

    fn create(seed: u64, settings: HillsSettings) -> Arc<Self> {
        let wavelength = settings.wavelength.max(50.0) as f64;
        Arc::new(Self {
            shape: Fractal::new(seed, "hills", 4, wavelength),
            detail: Fractal::new(seed, "hills detail", 3, wavelength / 6.0),
            settings,
        })
    }
}

impl TerrainSource for Hills {
    fn vertical_range(&self, (cx, cz): (i32, i32), _detail: bool) -> RangeInclusive<i32> {
        // Every 8 m over the square and a little past it; the noise does
        // not swing more than a few metres in between.
        let (x0, z0) = (cx as f64 * CHUNK_SIZE as f64, cz as f64 * CHUNK_SIZE as f64);
        let (mut low, mut high) = (f32::MAX, f32::MIN);
        for j in -2..=CHUNK_SIZE as i32 / 8 + 2 {
            for i in -2..=CHUNK_SIZE as i32 / 8 + 2 {
                let h = self.height(x0 + i as f64 * 8.0, z0 + j as f64 * 8.0);
                low = low.min(h);
                high = high.max(h);
            }
        }
        vertical_range_for(low - 6.0, high + 6.0)
    }

    fn chunk<'a>(&'a self, coord: ChunkCoord, _detail: bool) -> Box<dyn ChunkSurface + 'a> {
        Box::new(PlainSurface {
            field: ChunkField::from_fn(coord, |x, z| self.height(x, z)),
            surface: move |point: &SurfacePoint| SurfaceSample::plain(self.color(point.world[1] as f32)),
        })
    }

    fn surface_height(&self, x: f64, z: f64) -> f32 {
        self.height(x, z)
    }

    fn water_level(&self, _x: f64, _z: f64) -> f32 {
        self.sea()
    }

    fn sea_level(&self) -> Option<f32> {
        self.settings.sea.then_some(SEA_LEVEL)
    }

    fn horizon_sample(&self, x: f64, z: f64) -> HorizonSample {
        let height = self.height(x, z);
        HorizonSample {
            height,
            surface: SurfaceSample::plain(self.color(height)),
            water: self.sea(),
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hills_have_ground_and_sea_and_their_chunks_hold_it() {
        let hills = Hills::create(4, HillsSettings::default());
        let heights: Vec<f32> = (0..400).map(|i| hills.height(i as f64 * 37.0, i as f64 * -11.0)).collect();
        assert!(heights.iter().any(|&h| h > 10.0));
        let range = hills.vertical_range((0, 0), true);
        let h = hills.surface_height(64.0, 64.0);
        assert!(range.contains(&((h / CHUNK_SIZE as f32).floor() as i32)), "{range:?} {h}");
        // The field's heights are the ground's.
        let field = hills.chunk(ChunkCoord::new(0, 0, 0), true);
        assert_eq!(field.field().height(10, 20), hills.surface_height(10.0, 20.0));
    }
}
