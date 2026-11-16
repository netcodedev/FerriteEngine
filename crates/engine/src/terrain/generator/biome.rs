//! Biomes: what grows (and what the ground looks like) where, from the
//! climate at the surface. Like Minecraft's, a biome is a region of climate
//! space: temperature and humidity, plus the height (sea, shore, high
//! mountains).
//!
//! Neighbouring biomes blend: every biome gets a weight from how close the
//! climate is to its own, so ground colour and vegetation fade over a few
//! hundred metres instead of changing at a line.

use super::{world::smoothstep, SEA_LEVEL};

/// Degrees Celsius the air cools per metre of height. Steeper than the real
/// 6.5 °C per kilometre, so the world's mountains (up to about 2 km) reach
/// the snow.
pub const LAPSE_RATE: f32 = 0.012;

/// Mean temperature at height `h` (metres) where it is `sea_level` °C at
/// sea level.
pub fn temperature_at(sea_level: f32, h: f32) -> f32 {
    sea_level - LAPSE_RATE * (h - SEA_LEVEL).max(0.0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Biome {
    Ocean,
    Beach,
    Desert,
    Savanna,
    Steppe,
    Plains,
    Forest,
    Jungle,
    Swamp,
    Taiga,
    Tundra,
    Alpine,
    Glacier,
}

/// What a biome looks like and grows.
struct Traits {
    /// Ground colour (display space, as an artist would pick it).
    color: [f32; 3],
    /// How much of the scattered vegetation grows here, 0..1.
    vegetation: f32,
}

impl Biome {
    pub const ALL: [Biome; 13] = [
        Biome::Ocean,
        Biome::Beach,
        Biome::Desert,
        Biome::Savanna,
        Biome::Steppe,
        Biome::Plains,
        Biome::Forest,
        Biome::Jungle,
        Biome::Swamp,
        Biome::Taiga,
        Biome::Tundra,
        Biome::Alpine,
        Biome::Glacier,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Biome::Ocean => "ocean",
            Biome::Beach => "beach",
            Biome::Desert => "desert",
            Biome::Savanna => "savanna",
            Biome::Steppe => "steppe",
            Biome::Plains => "plains",
            Biome::Forest => "forest",
            Biome::Jungle => "jungle",
            Biome::Swamp => "swamp",
            Biome::Taiga => "taiga",
            Biome::Tundra => "tundra",
            Biome::Alpine => "alpine",
            Biome::Glacier => "glacier",
        }
    }

    fn traits(self) -> Traits {
        let (color, vegetation) = match self {
            Biome::Ocean => ([0.40, 0.36, 0.26], 0.0),
            Biome::Beach => ([0.80, 0.74, 0.54], 0.0),
            Biome::Desert => ([0.82, 0.68, 0.44], 0.02),
            Biome::Savanna => ([0.62, 0.58, 0.28], 0.15),
            Biome::Steppe => ([0.55, 0.56, 0.30], 0.08),
            Biome::Plains => ([0.30, 0.55, 0.16], 0.35),
            Biome::Forest => ([0.17, 0.42, 0.10], 1.0),
            Biome::Jungle => ([0.10, 0.44, 0.08], 1.0),
            Biome::Swamp => ([0.22, 0.32, 0.14], 0.6),
            Biome::Taiga => ([0.20, 0.33, 0.18], 0.8),
            Biome::Tundra => ([0.47, 0.46, 0.36], 0.03),
            Biome::Alpine => ([0.46, 0.47, 0.36], 0.0),
            Biome::Glacier => ([0.86, 0.88, 0.92], 0.0),
        };
        Traits { color, vegetation }
    }

    /// Where the biome sits in climate space: `(temperature °C, humidity)`
    /// and how far it reaches along each.
    fn centre(self) -> Option<((f32, f32), (f32, f32))> {
        Some(match self {
            Biome::Desert => ((24.0, 0.10), (5.0, 0.14)),
            Biome::Savanna => ((24.0, 0.42), (5.0, 0.16)),
            Biome::Steppe => ((11.0, 0.22), (6.0, 0.13)),
            Biome::Plains => ((14.0, 0.48), (6.0, 0.15)),
            Biome::Forest => ((12.0, 0.72), (6.0, 0.15)),
            Biome::Jungle => ((25.0, 0.85), (4.5, 0.15)),
            Biome::Swamp => ((17.0, 0.97), (6.0, 0.08)),
            Biome::Taiga => ((3.0, 0.62), (4.0, 0.25)),
            Biome::Tundra => ((-4.0, 0.45), (3.5, 0.40)),
            Biome::Alpine => ((-2.0, 0.50), (3.5, 0.60)),
            Biome::Glacier => ((-11.0, 0.50), (3.5, 0.80)),
            Biome::Ocean | Biome::Beach => return None,
        })
    }
}

/// The biomes at one spot, blended.
#[derive(Clone, Copy, Debug)]
pub struct BiomeBlend {
    /// The biome with the largest weight.
    pub dominant: Biome,
    /// Ground colour (display space).
    pub color: [f32; 3],
    /// How much vegetation grows, 0..1.
    pub vegetation: f32,
    /// Temperature at the surface, °C.
    pub temperature: f32,
    pub humidity: f32,
}

/// The biomes at a spot of height `h` (metres) whose climate at sea level
/// is `sea_temperature` (°C) and `humidity` (0..1).
pub fn biome_at(sea_temperature: f32, humidity: f32, h: f32) -> BiomeBlend {
    let temperature = temperature_at(sea_temperature, h);
    let mut weights = [0.0f32; Biome::ALL.len()];
    // Below the sea and on the shore, those win; the land's biomes fade in
    // above them.
    let ocean = 1.0 - smoothstep(SEA_LEVEL as f64 - 3.0, SEA_LEVEL as f64, h as f64) as f32;
    let shore = (1.0 - ocean) * (1.0 - smoothstep(3.0, 7.0, h as f64) as f32);
    // Swamps are lowland; the alpine biome is the bare land above the
    // trees, the tundra the same in the low, cold north.
    let lowland = 1.0 - smoothstep(25.0, 60.0, h as f64) as f32;
    let high = smoothstep(350.0, 700.0, h as f64) as f32;
    let mut land = [0.0f32; Biome::ALL.len()];
    for (i, biome) in Biome::ALL.iter().enumerate() {
        let Some(((t, hu), (st, sh))) = biome.centre() else {
            continue;
        };
        let (dt, dh) = ((temperature - t) / st, (humidity - hu) / sh);
        let mut w = (-(dt * dt + dh * dh)).exp();
        match biome {
            Biome::Swamp => w *= lowland,
            Biome::Alpine => w *= high,
            Biome::Tundra => w *= 1.0 - high,
            _ => {}
        }
        land[i] = w;
    }
    let total: f32 = land.iter().sum();
    let land_share = (1.0 - ocean) * (1.0 - shore);
    for (i, w) in land.iter().enumerate() {
        weights[i] = if total > 1e-12 { w / total * land_share } else { 0.0 };
    }
    if total <= 1e-12 {
        // Far from every centre (hotter or drier than any): the nearest
        // extreme.
        let fallback = if temperature > 10.0 { Biome::Desert } else { Biome::Glacier };
        weights[fallback as usize] = land_share;
    }
    weights[Biome::Ocean as usize] = ocean;
    weights[Biome::Beach as usize] = shore * (1.0 - ocean);

    let mut color = [0.0f32; 3];
    let mut vegetation = 0.0;
    let mut dominant = (Biome::Ocean, -1.0f32);
    for (biome, &w) in Biome::ALL.iter().zip(weights.iter()) {
        let traits = biome.traits();
        for k in 0..3 {
            color[k] += traits.color[k] * w;
        }
        vegetation += traits.vegetation * w;
        if w > dominant.1 {
            dominant = (*biome, w);
        }
    }
    BiomeBlend {
        dominant: dominant.0,
        color,
        vegetation,
        temperature,
        humidity,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn climates_give_their_biomes() {
        assert_eq!(biome_at(26.0, 0.05, 80.0).dominant, Biome::Desert);
        assert_eq!(biome_at(12.0, 0.75, 80.0).dominant, Biome::Forest);
        assert_eq!(biome_at(14.0, 0.48, 80.0).dominant, Biome::Plains);
        assert_eq!(biome_at(17.0, 0.98, 5.0 + 10.0).dominant, Biome::Swamp);
        assert_eq!(biome_at(3.0, 0.6, 50.0).dominant, Biome::Taiga);
        assert_eq!(biome_at(14.0, 0.5, -40.0).dominant, Biome::Ocean);
        assert_eq!(biome_at(14.0, 0.5, 1.5).dominant, Biome::Beach);
        // The same climate high up: above the trees, then ice.
        assert_eq!(biome_at(12.0, 0.6, 1450.0).dominant, Biome::Alpine);
        assert_eq!(biome_at(12.0, 0.6, 2400.0).dominant, Biome::Glacier);
    }

    #[test]
    fn weights_blend_smoothly() {
        // Small steps in climate make small steps in colour.
        let mut last = biome_at(-15.0, 0.0, 100.0).color;
        for i in 1..=400 {
            let t = -15.0 + i as f32 * 0.1;
            let now = biome_at(t, i as f32 / 400.0, 100.0).color;
            let step: f32 = (0..3).map(|k| (now[k] - last[k]).abs()).sum();
            assert!(step < 0.05, "at {t} °C: {last:?} -> {now:?}");
            last = now;
        }
    }

    #[test]
    fn vegetation_stops_above_the_trees() {
        assert!(biome_at(12.0, 0.75, 80.0).vegetation > 0.8);
        assert!(biome_at(12.0, 0.75, 1500.0).vegetation < 0.1);
        assert_eq!(biome_at(12.0, 0.75, -10.0).vegetation, 0.0);
    }
}
