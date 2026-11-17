//! How much the sun and the wind give: a solar panel's output from where
//! the sun stands, the clouds, rain, snow and shadows on it; the wind's
//! speed and a wind turbine's output.

use ferrite::prelude::*;

/// What the sky is doing, as the producers feel it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sky {
    /// Toward the sun (normalized).
    pub sun: Vector3<f32>,
    /// Low cloud cover as the sky shows it, 0..1.
    pub clouds: f32,
    /// How hard it rains, 0..1.
    pub rain: f32,
    /// How hard it snows, 0..1.
    pub snow: f32,
    /// How much snow lies on things, 0..1.
    pub snow_cover: f32,
}

impl Default for Sky {
    fn default() -> Self {
        Sky { sun: Vector3::unit_y(), clouds: 0.0, rain: 0.0, snow: 0.0, snow_cover: 0.0 }
    }
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How strong the sunlight is above the clouds, 0..1: none below the
/// horizon, weak while the sun is low (through more air).
pub fn daylight(sun: Vector3<f32>) -> f32 {
    smoothstep(-0.02, 0.25, sun.y)
}

/// What the weather lets through to a panel, 0..1 (clouds, rain, falling
/// snow, snow lying on the panel).
pub fn weather_factor(sky: &Sky) -> f32 {
    let clouds = 1.0 - 0.8 * sky.clouds.clamp(0.0, 1.0);
    let rain = 1.0 - 0.35 * sky.rain.clamp(0.0, 1.0);
    let snow = 1.0 - 0.5 * sky.snow.clamp(0.0, 1.0);
    // Snow lying on a tilted panel slides off in part.
    let cover = 1.0 - 0.85 * sky.snow_cover.clamp(0.0, 1.0);
    clouds * rain * snow * cover
}

/// What a solar panel of `watts` makes, its face looking along `normal`,
/// with `lit` of it (0..1) out of the shadow.
pub fn solar_watts(watts: f32, normal: Vector3<f32>, lit: f32, sky: &Sky) -> f32 {
    let facing = normal.dot(sky.sun).max(0.0);
    // Light scattered by the sky and clouds reaches a panel in the shade
    // or facing away a little.
    let diffuse = 0.12 * daylight(sky.sun) * (0.5 + 0.5 * normal.y);
    let direct = facing * daylight(sky.sun) * lit.clamp(0.0, 1.0);
    let clear = weather_factor(sky);
    // On an overcast day a little more of the light is diffuse.
    let diffuse = diffuse * (1.0 + sky.clouds);
    (watts * (direct * clear + diffuse * clear.max(0.3))).clamp(0.0, watts)
}

/// The wind's speed in metres per second, `seconds` into the game: gusts
/// of a few metres per second, stronger under clouds and much stronger in
/// rain and snow (a storm).
pub fn wind_speed(seconds: f32, sky: &Sky) -> f32 {
    let t = seconds;
    let calm = 5.0 + 2.2 * (t * 0.031).sin() + 1.4 * (t * 0.083 + 1.3).sin() + 0.7 * (t * 0.29 + 2.0).sin();
    let weather = 2.0 * sky.clouds + 7.0 * sky.rain.max(sky.snow);
    (calm + weather).max(0.0)
}

/// Wind speeds of a small turbine: it starts turning at `CUT_IN`, makes
/// its rated power from `RATED` and stops (turns out of the wind) above
/// `CUT_OUT` to protect itself.
pub const CUT_IN: f32 = 3.0;
pub const RATED: f32 = 12.0;
pub const CUT_OUT: f32 = 25.0;

/// What a wind turbine of `watts` makes in wind of `speed` m/s: the
/// wind's power grows with the cube of its speed.
pub fn wind_watts(watts: f32, speed: f32) -> f32 {
    if speed < CUT_IN || speed > CUT_OUT {
        0.0
    } else if speed >= RATED {
        watts
    } else {
        watts * (speed.powi(3) - CUT_IN.powi(3)) / (RATED.powi(3) - CUT_IN.powi(3))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sun(altitude_deg: f32) -> Vector3<f32> {
        let a = altitude_deg.to_radians();
        Vector3::new(a.cos(), a.sin(), 0.0)
    }

    #[test]
    fn a_panel_facing_the_noon_sun_gives_its_rated_power() {
        let sky = Sky { sun: sun(90.0), ..Sky::default() };
        let w = solar_watts(400.0, Vector3::unit_y(), 1.0, &sky);
        assert!(w > 380.0, "{w}");
    }

    #[test]
    fn nothing_at_night() {
        let sky = Sky { sun: sun(-20.0), ..Sky::default() };
        assert_eq!(solar_watts(400.0, Vector3::unit_y(), 1.0, &sky), 0.0);
    }

    #[test]
    fn clouds_rain_and_snow_cut_the_output() {
        let clear = Sky { sun: sun(70.0), ..Sky::default() };
        let w = |sky: &Sky| solar_watts(400.0, Vector3::unit_y(), 1.0, sky);
        let cloudy = Sky { clouds: 0.8, ..clear };
        let rainy = Sky { clouds: 0.9, rain: 1.0, ..clear };
        let snowed = Sky { snow_cover: 1.0, ..clear };
        assert!(w(&cloudy) < w(&clear) * 0.5);
        assert!(w(&rainy) < w(&cloudy));
        assert!(w(&snowed) < w(&clear) * 0.25);
        assert!(w(&rainy) > 0.0);
    }

    #[test]
    fn shadow_on_the_panel_cuts_the_output() {
        let sky = Sky { sun: sun(60.0), ..Sky::default() };
        let full = solar_watts(400.0, Vector3::unit_y(), 1.0, &sky);
        let half = solar_watts(400.0, Vector3::unit_y(), 0.5, &sky);
        let shade = solar_watts(400.0, Vector3::unit_y(), 0.0, &sky);
        assert!(half < full * 0.6 && shade < full * 0.2 && shade > 0.0);
    }

    #[test]
    fn a_panel_turned_away_from_the_sun_gives_less() {
        let sky = Sky { sun: sun(30.0), ..Sky::default() };
        let toward = solar_watts(400.0, Vector3::new(0.5, 0.866, 0.0), 1.0, &sky);
        let away = solar_watts(400.0, Vector3::new(-0.5, 0.866, 0.0), 1.0, &sky);
        assert!(toward > away * 2.0);
    }

    #[test]
    fn the_turbine_follows_the_cube_of_the_wind() {
        assert_eq!(wind_watts(3000.0, 2.0), 0.0);
        assert_eq!(wind_watts(3000.0, 15.0), 3000.0);
        assert_eq!(wind_watts(3000.0, 30.0), 0.0);
        let half = wind_watts(3000.0, 6.0);
        assert!(half > 0.0 && half < 600.0);
        assert!(wind_watts(3000.0, 9.0) > half * 2.0);
    }

    #[test]
    fn storms_blow_harder() {
        let calm = Sky::default();
        let storm = Sky { rain: 1.0, clouds: 1.0, ..calm };
        for t in [0.0, 100.0, 1000.0] {
            assert!(wind_speed(t, &storm) > wind_speed(t, &calm) + 5.0);
            assert!(wind_speed(t, &calm) > 0.0);
        }
    }
}
