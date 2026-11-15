//! Light that does not come straight from a lamp or the sun: the moon at
//! night, the glow of the night sky, and lamp light bounced off the
//! surroundings. It keeps nights from going black and lets a lantern light
//! a whole room, not only what it shines on.
//!
//! This is an approximation, not traced light: the moon is a second,
//! dimmer sun (it gets the sun's shadow cascades while the sun is down),
//! the night sky raises the ambient light, and each lamp adds a soft,
//! wide term to the ambient light around it (see `localBounce` in
//! `pbr/lighting.glsl`). Ambient occlusion still darkens all of it in
//! corners and under things.

use cgmath::{InnerSpace, Vector3};
use serde::{Deserialize, Serialize};

/// The sun is this far below the horizon (direction y) before the moon
/// takes over the shadow cascades. The sun gives no light from here on
/// (see `sunRadiance`), and the moon none before (see `moonRadiance`), so
/// the switch shows no jump.
pub const MOON_SHADOW_BELOW: f32 = -0.06;
/// How far the moon's path is tilted off the sun's, so it is not exactly
/// opposite it (direction z).
const MOON_TILT: f32 = 0.35;

/// Tunables of the indirect light, kept with the sky light.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(default)]
pub struct GlobalIllumination {
    /// Brightness of the moon, 0 for moonless nights. 1 is the default
    /// moon, a soft blue light about a tenth of the sun's.
    pub moonlight: f32,
    /// Ambient light of the night sky (stars, airglow): how dark nights get
    /// where the moon does not reach. 1 is the default.
    pub night_sky: f32,
    /// How much lamp light bounces off the surroundings onto everything
    /// near the lamp. Each light scales it with its own `bounce`.
    pub bounce: f32,
}

impl Default for GlobalIllumination {
    fn default() -> Self {
        Self {
            moonlight: 1.0,
            night_sky: 1.0,
            bounce: 1.0,
        }
    }
}

impl GlobalIllumination {
    pub const MOONLIGHT_RANGE: (f32, f32) = (0.0, 4.0);
    pub const NIGHT_SKY_RANGE: (f32, f32) = (0.0, 8.0);
    pub const BOUNCE_RANGE: (f32, f32) = (0.0, 4.0);

    pub fn set_moonlight(&mut self, v: f32) {
        self.moonlight = clamp(v, Self::MOONLIGHT_RANGE, 1.0);
    }

    pub fn set_night_sky(&mut self, v: f32) {
        self.night_sky = clamp(v, Self::NIGHT_SKY_RANGE, 1.0);
    }

    pub fn set_bounce(&mut self, v: f32) {
        self.bounce = clamp(v, Self::BOUNCE_RANGE, 1.0);
    }

    /// Every field through its setter (for hand edited scene files).
    pub fn sanitized(self) -> Self {
        let mut s = Self::default();
        s.set_moonlight(self.moonlight);
        s.set_night_sky(self.night_sky);
        s.set_bounce(self.bounce);
        s
    }
}

fn clamp(v: f32, (min, max): (f32, f32), fallback: f32) -> f32 {
    if v.is_finite() {
        v.clamp(min, max)
    } else {
        fallback
    }
}

/// Toward the moon, for the sun toward `sun`: across the sky from it, its
/// path tilted a little to the side.
pub fn moon_direction(sun: Vector3<f32>) -> Vector3<f32> {
    Vector3::new(-sun.x, -sun.y, -sun.z + MOON_TILT).normalize()
}

/// Whether the moon casts the shadows (the sun is down).
pub fn moon_casts_shadows(sun: Vector3<f32>) -> bool {
    sun.y < MOON_SHADOW_BELOW
}

/// The light and settings the lit shaders are given (`Lighting::apply`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IndirectShading {
    pub moon_direction: Vector3<f32>,
    pub moon_shadows: bool,
    pub settings: GlobalIllumination,
}

impl IndirectShading {
    pub fn for_sun(sun: Vector3<f32>, settings: GlobalIllumination) -> Self {
        Self {
            moon_direction: moon_direction(sun),
            moon_shadows: moon_casts_shadows(sun),
            settings,
        }
    }

    /// Previews: no moon or bounce, the default night sky.
    pub fn preview(sun: Vector3<f32>) -> Self {
        Self::for_sun(
            sun,
            GlobalIllumination {
                moonlight: 0.0,
                night_sky: 1.0,
                bounce: 0.0,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_moon_is_up_while_the_sun_is_down() {
        let midnight_sun = Vector3::new(0.0, -1.0, 0.0);
        let moon = moon_direction(midnight_sun);
        assert!(moon.y > 0.9);
        assert!((moon.magnitude() - 1.0).abs() < 1e-6);
        let noon_sun = Vector3::new(0.0, 1.0, 0.0);
        assert!(moon_direction(noon_sun).y < -0.9);
    }

    #[test]
    fn the_moon_takes_the_shadows_only_once_the_sun_has_set() {
        assert!(!moon_casts_shadows(Vector3::new(1.0, 0.0, 0.0)));
        assert!(!moon_casts_shadows(Vector3::new(1.0, -0.05, 0.0).normalize()));
        assert!(moon_casts_shadows(Vector3::new(1.0, -0.2, 0.0).normalize()));
    }

    #[test]
    fn settings_are_clamped() {
        let s = GlobalIllumination {
            moonlight: -1.0,
            night_sky: 100.0,
            bounce: f32::NAN,
        }
        .sanitized();
        assert_eq!(s.moonlight, 0.0);
        assert_eq!(s.night_sky, GlobalIllumination::NIGHT_SKY_RANGE.1);
        assert_eq!(s.bounce, 1.0);
    }

    #[test]
    fn older_scenes_get_the_defaults() {
        let s: GlobalIllumination = ron::from_str("(moonlight: 0.5)").unwrap();
        assert_eq!(s.moonlight, 0.5);
        assert_eq!(s.night_sky, 1.0);
        assert_eq!(s.bounce, 1.0);
    }
}
