//! Pictures of the whole world from above, for tuning it: the shaded
//! height or one of the climate maps.

use image::{Rgba, RgbaImage};

use super::{
    biome::{biome_at, temperature_at},
    WorldGenerator, SEA_LEVEL,
};

/// What a map shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapLayer {
    /// The land in colours by height, sea in blues by depth, lit from the
    /// north-west.
    Height,
    /// The biomes' ground colours, lit like the height map, snow white.
    Biomes,
    /// Mean temperature at the ground: blue cold, red hot.
    Temperature,
    /// Brown dry, green, blue wet.
    Humidity,
    Continentalness,
    Erosion,
    Ridges,
}

impl MapLayer {
    pub const ALL: [MapLayer; 7] = [
        MapLayer::Height,
        MapLayer::Biomes,
        MapLayer::Temperature,
        MapLayer::Humidity,
        MapLayer::Continentalness,
        MapLayer::Erosion,
        MapLayer::Ridges,
    ];

    pub fn name(self) -> &'static str {
        match self {
            MapLayer::Height => "height",
            MapLayer::Biomes => "biomes",
            MapLayer::Temperature => "temperature",
            MapLayer::Humidity => "humidity",
            MapLayer::Continentalness => "continentalness",
            MapLayer::Erosion => "erosion",
            MapLayer::Ridges => "ridges",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|layer| layer.name() == name)
    }
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

/// Colour of land or sea at height `h` (metres).
pub fn height_color(h: f32) -> [f32; 3] {
    const STOPS: [(f32, [f32; 3]); 11] = [
        (-420.0, [0.05, 0.10, 0.28]),
        (-150.0, [0.10, 0.22, 0.48]),
        (-20.0, [0.22, 0.45, 0.70]),
        (0.0, [0.45, 0.68, 0.82]),
        (0.01, [0.80, 0.76, 0.56]),
        (8.0, [0.42, 0.62, 0.32]),
        (150.0, [0.30, 0.50, 0.24]),
        (400.0, [0.55, 0.55, 0.36]),
        (800.0, [0.55, 0.45, 0.36]),
        (1200.0, [0.62, 0.60, 0.58]),
        (1700.0, [0.96, 0.96, 0.98]),
    ];
    let i = STOPS.partition_point(|s| s.0 <= h);
    if i == 0 {
        return STOPS[0].1;
    }
    if i == STOPS.len() {
        return STOPS[STOPS.len() - 1].1;
    }
    let (a, b) = (STOPS[i - 1], STOPS[i]);
    mix(a.1, b.1, (h - a.0) / (b.0 - a.0))
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A diverging colour for a value in `-1..1`.
fn signed_color(v: f32) -> [f32; 3] {
    if v < 0.0 {
        mix([0.95, 0.95, 0.95], [0.15, 0.30, 0.75], -v)
    } else {
        mix([0.95, 0.95, 0.95], [0.75, 0.25, 0.10], v)
    }
}

impl WorldGenerator {
    /// The world (its full size, plus a little sea around it) as a square
    /// picture `pixels` wide, north (−z) up. Uses all cores.
    pub fn map_image(&self, pixels: u32, layer: MapLayer) -> RgbaImage {
        let half = self.settings().size * 0.55;
        let step = 2.0 * half / pixels as f64;
        let n = pixels as usize;
        let mut values = vec![[0.0f32; 3]; n * n];
        // Heights one pixel beyond, for the hill shading.
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
        let rows_per = n.div_ceil(threads);
        std::thread::scope(|scope| {
            for (t, chunk) in values.chunks_mut(rows_per * n).enumerate() {
                scope.spawn(move || {
                    for (k, v) in chunk.iter_mut().enumerate() {
                        let (row, col) = (t * rows_per + k / n, k % n);
                        let x = -half + (col as f64 + 0.5) * step;
                        let z = -half + (row as f64 + 0.5) * step;
                        // Lit from the north-west.
                        let shaded = |h: f32| {
                            let hx = self.coarse_height(x + step, z) - self.coarse_height(x - step, z);
                            let hz = self.coarse_height(x, z + step) - self.coarse_height(x, z - step);
                            if h > SEA_LEVEL {
                                let slope_light = (-(hx + hz) as f64 / (2.0 * step) * 1.5) as f32;
                                (0.85 + slope_light * 0.5).clamp(0.45, 1.25)
                            } else {
                                1.0
                            }
                        };
                        // Rivers, lakes, roads and settlements over the ground, on the
                        // maps of it.
                        let water = |ground: [f32; 3]| -> [f32; 3] {
                            // Settlements and roads over everything.
                            if let Some(works) = self.roads().and_then(|r| {
                                let c = |v: f64| (v / crate::terrain::CHUNK_SIZE as f64).floor() as i32;
                                r.near(c(x), c(z))
                            }) {
                                let near = |s: &super::roads::Settlement| {
                                    (x - s.centre[0]).hypot(z - s.centre[1]) < (s.radius as f64).max(step * 1.2)
                                };
                                if works.settlements.iter().any(near) {
                                    return [0.78, 0.22, 0.14];
                                }
                                let road = super::roads::Roads::nearest(&works.roads, x, z);
                                if road.is_some_and(|r| (r.distance as f64) < step * 0.5) {
                                    return [0.30, 0.18, 0.10];
                                }
                                // Bridges paler.
                                if works.bridges.iter().any(|b| b.covers(x, z, (step * 0.5) as f32 - 4.5)) {
                                    return [0.62, 0.56, 0.48];
                                }
                                // Tunnels dashed, lighter: under the ground.
                                let tunnel = super::roads::Roads::nearest(&works.tunnels, x, z);
                                let dash = ((x / step).floor() + (z / step).floor()).rem_euclid(3.0) < 2.0;
                                if dash && tunnel.is_some_and(|r| (r.distance as f64) < step * 0.5) {
                                    return [0.52, 0.46, 0.40];
                                }
                            }
                            let Some(plan) = self.hydrology() else {
                                return ground;
                            };
                            if plan.lake_at(x, z).is_some_and(|(_, inside)| inside > 0.45) {
                                [0.16, 0.36, 0.62]
                            } else if plan.river_near(x, z, step * 0.5) {
                                [0.18, 0.42, 0.72]
                            } else {
                                ground
                            }
                        };
                        *v = match layer {
                            MapLayer::Height => {
                                let h = self.coarse_height(x, z);
                                water(height_color(h).map(|c| c * shaded(h)))
                            }
                            MapLayer::Biomes => {
                                let h = self.coarse_height(x, z);
                                let c = self.climate(x, z);
                                let biome = biome_at(c.temperature, c.humidity, h);
                                let ground = if h < SEA_LEVEL {
                                    height_color(h)
                                } else {
                                    // Snow as the terrain's shader lays it.
                                    let snow = 1.0 - smoothstep(-6.0, -3.5, biome.temperature);
                                    mix(biome.color, [0.86, 0.88, 0.92], snow)
                                };
                                water(ground.map(|c| c * shaded(h)))
                            }
                            MapLayer::Temperature => {
                                let h = self.coarse_height(x, z);
                                let t = temperature_at(self.climate(x, z).temperature, h);
                                signed_color(((t - 10.0) / 20.0).clamp(-1.0, 1.0))
                            }
                            MapLayer::Humidity => {
                                let u = self.climate(x, z).humidity;
                                if u < 0.5 {
                                    mix([0.62, 0.45, 0.25], [0.35, 0.62, 0.25], u * 2.0)
                                } else {
                                    mix([0.35, 0.62, 0.25], [0.15, 0.35, 0.75], u * 2.0 - 1.0)
                                }
                            }
                            MapLayer::Continentalness => signed_color(self.climate(x, z).continentalness),
                            MapLayer::Erosion => signed_color(self.climate(x, z).erosion),
                            MapLayer::Ridges => {
                                let r = self.climate(x, z).ridges;
                                mix([0.1, 0.1, 0.12], [1.0, 0.95, 0.85], r)
                            }
                        };
                    }
                });
            }
        });
        let mut image = RgbaImage::new(pixels, pixels);
        for (i, v) in values.iter().enumerate() {
            let rgb = v.map(|c| (c.clamp(0.0, 1.0) * 255.0) as u8);
            image.put_pixel((i % n) as u32, (i / n) as u32, Rgba([rgb[0], rgb[1], rgb[2], 255]));
        }
        image
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_map_shows_sea_at_its_edge_and_land_in_the_middle() {
        let generator = WorldGenerator::new(2);
        let image = generator.map_image(48, MapLayer::Height);
        let corner = image.get_pixel(0, 0).0;
        let middle = image.get_pixel(24, 24).0;
        // Deep sea is blue, the spawn area is not.
        assert!(corner[2] > corner[0] && corner[2] > corner[1], "{corner:?}");
        assert!(middle[2] < middle[1], "{middle:?}");
        for layer in MapLayer::ALL {
            assert_eq!(MapLayer::from_name(layer.name()), Some(layer));
        }
    }

    /// Writes maps of a few seeds to look at:
    /// `cargo test -p ferrite world_maps -- --ignored` (to `target/world-maps`).
    #[test]
    #[ignore]
    fn world_maps() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/world-maps");
        std::fs::create_dir_all(&dir).unwrap();
        for seed in [1, 2, 3] {
            let generator = WorldGenerator::new(seed);
            for layer in MapLayer::ALL {
                generator
                    .map_image(512, layer)
                    .save(dir.join(format!("seed{seed}-{}.png", layer.name())))
                    .unwrap();
            }
        }
    }
}
