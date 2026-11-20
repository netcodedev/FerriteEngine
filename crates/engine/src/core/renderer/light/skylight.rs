use cgmath::{ortho, EuclideanSpace, InnerSpace, Matrix4, Point3, SquareMatrix, Vector3, Vector4};
use glfw::{Glfw, WindowEvent};

use crate::core::{
    camera::{Camera, Projection},
    entity::{
        component::{Component, Property, PropertyValue},
        Entity,
    },
    renderer::{
        hdr::{HdrSettings, Tonemapper},
        light::{
            global_illumination::{
                moon_casts_shadows, moon_direction, GlobalIllumination, IndirectShading,
            },
            light::clamp_finite,
        },
        sky::{CloudSettings, CloudType},
        ssao::AmbientOcclusionSettings,
        weather::{Weather, WeatherMode},
    },
    scene::{scene_file::ComponentData, Scene},
};

/// Number of shadow cascades; must match `SHADOW_CASCADES` in
/// `pbr/lighting.glsl`.
pub const SHADOW_CASCADES: usize = 4;
/// How far from the camera shadows reach.
const SHADOW_DISTANCE: f32 = 300.0;
/// Mix between logarithmic (1) and even (0) cascade splits.
const SPLIT_LAMBDA: f32 = 0.75;
/// Shadow casters this far toward the sun beyond a cascade still cast into
/// it (tall trees, hills between the sun and the view).
const CASTER_MARGIN: f32 = 150.0;

/// Default real-time length of one in-game day, in seconds.
const DEFAULT_DAY_LENGTH_SECONDS: f32 = 240.0;
/// Sun radius used to position the directional light away from the scene
/// origin. The exact value doesn't matter — only the direction is used for
/// lighting — but keeping it large avoids precision issues in the shadow
/// matrix when the light passes near the zenith.
const SUN_DISTANCE: f32 = 200.0;

pub struct SkyLight {
    position: Point3<f32>,
    cascades: [ShadowCascade; SHADOW_CASCADES],

    /// Current time of day, in [0, 1). 0.0 == midnight, 0.5 == solar noon.
    time_of_day: f32,
    /// Seconds of real time per one full simulated day.
    day_length_seconds: f32,
    /// When true the cycle is frozen — manual F5/F6 steps still apply.
    paused: bool,
    /// Cloud coverage in [0, 1]. 0 ≈ clear sky, 1 ≈ overcast.
    cloud_coverage: f32,
    /// The low layer's cloud type and the layers above it.
    clouds: CloudSettings,
    /// How the sky light is shaded off in corners and crevices.
    ambient_occlusion: AmbientOcclusionSettings,
    /// Exposure, tone mapping and bloom of the camera's view.
    hdr: HdrSettings,
    /// Rain and snow, and what they left on the ground.
    weather: Weather,
    /// Moonlight, the night sky's light and lamp light bounce.
    global_illumination: GlobalIllumination,
    /// How often the shadow cascades are drawn.
    shadows: ShadowSettings,
}

/// How often the sun's shadow cascades are drawn again.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Default)]
#[serde(default)]
pub struct ShadowSettings {
    /// Draw the second cascade every other frame (the third and fourth are
    /// drawn every 2nd and 4th frame anyway). Saves about a quarter of the
    /// shadow pass; a fast turn or move can show its shadows a frame late.
    pub stagger_second_cascade: bool,
}

impl ShadowSettings {
    /// Every how many frames each cascade is drawn, nearest first.
    pub fn intervals(&self) -> [u64; SHADOW_CASCADES] {
        [1, if self.stagger_second_cascade { 2 } else { 1 }, 2, 4]
    }
}

impl SkyLight {
    /// Construct a sky light. The initial direction is derived from the
    /// supplied `position` vector (only its direction matters; magnitude is
    /// normalised). The time-of-day defaults to morning (~8 AM).
    pub fn new<P: Into<Point3<f32>>>(position: P) -> Self {
        let p: Point3<f32> = position.into();
        // Recover an initial time-of-day from the seed position so a caller
        // can still hint at "morning sun in the east" with `(30, 20, 10)`.
        let dir = p.to_vec();
        let initial_time = if dir.x.abs() + dir.y.abs() < 0.0001 {
            8.0 / 24.0
        } else {
            // angle = time*2π - π/2  →  time = (atan2(y, x) + π/2) / 2π
            let angle = dir.y.atan2(dir.x);
            let mut t = (angle + std::f32::consts::FRAC_PI_2) / std::f32::consts::TAU;
            if t < 0.0 {
                t += 1.0;
            }
            t
        };

        let mut sl = Self {
            position: p,
            cascades: [ShadowCascade {
                view_projection: Matrix4::identity(),
                texel_size: 0.0,
            }; SHADOW_CASCADES],
            time_of_day: initial_time,
            day_length_seconds: DEFAULT_DAY_LENGTH_SECONDS,
            paused: false,
            cloud_coverage: 0.2,
            clouds: CloudSettings::default(),
            ambient_occlusion: AmbientOcclusionSettings::default(),
            hdr: HdrSettings::default(),
            weather: Weather::default(),
            global_illumination: GlobalIllumination::default(),
            shadows: ShadowSettings::default(),
        };
        sl.position = Point3::from_vec(sl.compute_sun_direction() * SUN_DISTANCE);
        sl
    }

    /// Returns the world-space unit vector pointing from the origin toward
    /// the sun. At t=0.25 (sunrise) it points along +X; at t=0.5 (noon) it
    /// points along +Y; at t=0.75 (sunset) it points along -X.
    pub fn compute_sun_direction(&self) -> Vector3<f32> {
        let angle = self.time_of_day * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
        Vector3::new(angle.cos(), angle.sin(), 0.0).normalize()
    }

    pub fn get_sun_direction(&self) -> Vector3<f32> {
        self.compute_sun_direction()
    }

    /// Toward the moon (see `global_illumination::moon_direction`).
    pub fn get_moon_direction(&self) -> Vector3<f32> {
        moon_direction(self.compute_sun_direction())
    }

    /// Toward the light the shadow cascades are cast from: the sun by day,
    /// the moon once the sun has set.
    pub fn shadow_light_direction(&self) -> Vector3<f32> {
        let sun = self.compute_sun_direction();
        if moon_casts_shadows(sun) {
            moon_direction(sun)
        } else {
            sun
        }
    }

    pub fn global_illumination(&self) -> &GlobalIllumination {
        &self.global_illumination
    }

    pub fn set_global_illumination(&mut self, settings: GlobalIllumination) {
        self.global_illumination = settings.sanitized();
    }

    pub fn shadows(&self) -> &ShadowSettings {
        &self.shadows
    }

    pub fn set_shadows(&mut self, settings: ShadowSettings) {
        self.shadows = settings;
    }

    /// The moon and indirect light as the lit shaders get them.
    pub fn indirect_shading(&self) -> IndirectShading {
        IndirectShading::for_sun(self.compute_sun_direction(), self.global_illumination)
    }

    pub fn get_time_of_day(&self) -> f32 {
        self.time_of_day
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    pub fn toggle_paused(&mut self) {
        self.paused = !self.paused;
    }

    pub fn get_cloud_coverage(&self) -> f32 {
        self.cloud_coverage
    }

    pub fn set_cloud_coverage(&mut self, c: f32) {
        self.cloud_coverage = clamp_finite(c, (0.0, 1.0), self.cloud_coverage);
    }

    pub fn adjust_cloud_coverage(&mut self, delta: f32) {
        self.set_cloud_coverage(self.cloud_coverage + delta);
    }

    pub fn clouds(&self) -> &CloudSettings {
        &self.clouds
    }

    /// Coverage of the low clouds as the sky shows it: the set coverage,
    /// closed toward overcast while it rains or snows.
    pub fn effective_cloud_coverage(&self) -> f32 {
        self.weather.cloud_coverage(self.cloud_coverage)
    }

    /// Shape of the low clouds as the sky shows it (the `cloudType`
    /// uniform): the set type, turned toward the weather's clouds.
    pub fn effective_cloud_type(&self) -> f32 {
        self.weather.cloud_type(self.clouds.cloud_type.shader_value())
    }

    pub fn weather(&self) -> &Weather {
        &self.weather
    }

    pub fn weather_mut(&mut self) -> &mut Weather {
        &mut self.weather
    }

    pub fn set_weather(&mut self, weather: Weather) {
        self.weather = weather.sanitized();
    }

    /// How strongly the sun shines on the ground, 0..1 (dries and melts).
    fn sunshine(&self) -> f32 {
        let height = self.compute_sun_direction().y;
        let up = ((height + 0.05) / 0.35).clamp(0.0, 1.0);
        up * (1.0 - 0.85 * self.cloud_coverage)
    }

    pub fn set_clouds(&mut self, settings: CloudSettings) {
        self.clouds = settings.sanitized();
    }

    pub fn ambient_occlusion(&self) -> &AmbientOcclusionSettings {
        &self.ambient_occlusion
    }

    pub fn set_ambient_occlusion(&mut self, settings: AmbientOcclusionSettings) {
        self.ambient_occlusion.enabled = settings.enabled;
        self.ambient_occlusion.set_radius(settings.radius);
        self.ambient_occlusion.set_intensity(settings.intensity);
    }

    pub fn hdr(&self) -> &HdrSettings {
        &self.hdr
    }

    pub fn set_hdr(&mut self, settings: HdrSettings) {
        self.hdr = settings.sanitized();
    }

    /// Advance (or rewind) the clock by `delta` of a day. `delta = 1/24`
    /// equals one hour. Wraps within [0, 1).
    pub fn advance_time(&mut self, delta: f32) {
        let mut t = self.time_of_day + delta;
        t = t.rem_euclid(1.0);
        self.time_of_day = t;
        self.position = Point3::from_vec(self.compute_sun_direction() * SUN_DISTANCE);
    }

    /// (hours, minutes) in 24-hour clock representation.
    pub fn get_clock(&self) -> (u32, u32) {
        let total_minutes = (self.time_of_day * 24.0 * 60.0) as u32;
        ((total_minutes / 60) % 24, total_minutes % 60)
    }

    /// Fit the shadow cascades to the camera's view, each covering one
    /// slice of it, for a shadow map of `resolution` texels per side.
    pub fn update_light_view(&mut self, camera: &Camera, projection: &Projection, resolution: u32) {
        let eye = camera.get_eye_position();
        let inv_view = camera.get_matrix().invert().unwrap();
        let axis = |v: Vector4<f32>| (inv_view * v).truncate().normalize();
        let forward = axis(Vector4::new(0.0, 0.0, -1.0, 0.0));
        let up = axis(Vector4::new(0.0, 1.0, 0.0, 0.0));
        let right = axis(Vector4::new(1.0, 0.0, 0.0, 0.0));
        let tan_y = (projection.fovy.0 / 2.0).tan();
        let tan_x = tan_y * projection.aspect;

        // Rotation into light space: looking along the sunlight (the
        // moonlight at night).
        let light_direction = -self.shadow_light_direction();
        let light_up = if light_direction.x.abs() < 0.001 && light_direction.z.abs() < 0.001 {
            Vector3::unit_z()
        } else {
            Vector3::unit_y()
        };
        let light_view =
            Matrix4::look_at_rh(Point3::origin(), Point3::from_vec(light_direction), light_up);

        let splits = cascade_splits(projection.znear, SHADOW_DISTANCE);
        let mut near = projection.znear;
        for (cascade, far) in self.cascades.iter_mut().zip(splits) {
            let corners: Vec<Point3<f32>> = [near, far]
                .iter()
                .flat_map(|&d| {
                    let c = eye + forward * d;
                    let (h, w) = (up * tan_y * d, right * tan_x * d);
                    [c + h - w, c + h + w, c - h - w, c - h + w]
                })
                .collect();
            let center = Point3::centroid(&corners);
            // A sphere around the slice keeps the cascade the same size
            // whichever way the camera turns, so its texels do not swim.
            let radius = corners
                .iter()
                .map(|c| (c - center).magnitude())
                .fold(0.0, f32::max);
            let radius = (radius * 16.0).ceil() / 16.0;
            let texel = 2.0 * radius / resolution as f32;

            // Move the cascade in whole texels only, for the same reason.
            let c = light_view * center.to_homogeneous();
            let (cx, cy) = ((c.x / texel).floor() * texel, (c.y / texel).floor() * texel);
            let depth = -c.z;
            let projection = ortho(
                cx - radius,
                cx + radius,
                cy - radius,
                cy + radius,
                depth - radius - CASTER_MARGIN,
                depth + radius,
            );
            *cascade = ShadowCascade {
                view_projection: projection * light_view,
                texel_size: texel,
            };
            near = far;
        }
    }

    /// The shadow cascades, nearest first.
    pub fn shadow_cascades(&self) -> &[ShadowCascade; SHADOW_CASCADES] {
        &self.cascades
    }

    pub fn get_position(&self) -> Point3<f32> {
        self.position
    }

}

impl SkyLight {
    pub fn set_time_of_day(&mut self, time_of_day: f32) {
        if !time_of_day.is_finite() {
            return;
        }
        self.time_of_day = time_of_day.rem_euclid(1.0);
        self.position = Point3::from_vec(self.compute_sun_direction() * SUN_DISTANCE);
    }

    pub fn get_day_length_seconds(&self) -> f32 {
        self.day_length_seconds
    }

    pub fn set_day_length_seconds(&mut self, seconds: f32) {
        self.day_length_seconds = seconds.max(0.0);
    }

    fn refit_to_active_camera(&mut self, scene: &Scene) {
        if let (Some(camera_component), Some((resolution, _))) =
            (scene.get_active_camera(), scene.get_shadow_map_size())
        {
            let camera = camera_component.get_camera();
            let projection = camera_component.get_projection();
            self.update_light_view(camera, projection, resolution);
        }
    }
}

impl Component for SkyLight {
    fn update(&mut self, scene: &mut Scene, _: &mut Entity, delta_time: f64) {
        if !self.paused && self.day_length_seconds > 0.0 {
            let dt_day = (delta_time as f32) / self.day_length_seconds;
            self.advance_time(dt_day);
        }
        let sunshine = self.sunshine();
        self.weather.update(delta_time as f32, sunshine);
        self.refit_to_active_camera(scene);
    }

    fn update_in_editor(&mut self, scene: &mut Scene, _: &mut Entity, _: f64) {
        // Time stays frozen while editing; only the shadow box follows the
        // editor camera so shadows stay visible where you are looking.
        // The sky shows the chosen weather at once; what lies on the ground
        // is set by hand (wetness, snow cover, ...).
        if self.weather.mode != WeatherMode::Changing {
            self.weather.settle();
        }
        self.refit_to_active_camera(scene);
    }

    fn handle_event(&mut self, _: &mut Glfw, _: &mut glfw::Window, _: &WindowEvent) {}

    fn display_name(&self) -> String {
        "Sky light".into()
    }

    fn to_data(&self) -> Option<ComponentData> {
        Some(ComponentData::SkyLight {
            time_of_day: self.time_of_day,
            day_length_seconds: self.day_length_seconds,
            paused: self.paused,
            cloud_coverage: self.cloud_coverage,
            clouds: self.clouds,
            ambient_occlusion: self.ambient_occlusion,
            hdr: self.hdr,
            weather: self.weather,
            global_illumination: self.global_illumination,
            shadows: self.shadows,
        })
    }

    fn properties(&self) -> Vec<Property> {
        vec![
            Property::ranged("time of day", self.time_of_day, 0.0, 1.0),
            Property::float("day length (s)", self.day_length_seconds),
            Property::bool("paused", self.paused),
            Property::ranged("cloud coverage", self.cloud_coverage, 0.0, 1.0),
            Property::choice(
                "cloud type",
                self.clouds.cloud_type.name().to_string(),
                CloudType::ALL.iter().map(|t| t.name().to_string()).collect(),
            ),
            Property::ranged("altocumulus", self.clouds.altocumulus, 0.0, 1.0),
            Property::ranged("cirrus", self.clouds.cirrus, 0.0, 1.0),
            Property::bool("cloud reprojection", self.clouds.reprojection),
            Property::bool("staggered near shadows", self.shadows.stagger_second_cascade),
            Property::ranged(
                "moonlight",
                self.global_illumination.moonlight,
                GlobalIllumination::MOONLIGHT_RANGE.0,
                GlobalIllumination::MOONLIGHT_RANGE.1,
            ),
            Property::ranged(
                "night sky light",
                self.global_illumination.night_sky,
                GlobalIllumination::NIGHT_SKY_RANGE.0,
                GlobalIllumination::NIGHT_SKY_RANGE.1,
            ),
            Property::ranged(
                "light bounce",
                self.global_illumination.bounce,
                GlobalIllumination::BOUNCE_RANGE.0,
                GlobalIllumination::BOUNCE_RANGE.1,
            ),
            Property::bool("ambient occlusion", self.ambient_occlusion.enabled),
            Property::ranged(
                "AO radius",
                self.ambient_occlusion.radius,
                AmbientOcclusionSettings::RADIUS_RANGE.0,
                AmbientOcclusionSettings::RADIUS_RANGE.1,
            ),
            Property::ranged(
                "AO intensity",
                self.ambient_occlusion.intensity,
                AmbientOcclusionSettings::INTENSITY_RANGE.0,
                AmbientOcclusionSettings::INTENSITY_RANGE.1,
            ),
            Property::choice(
                "tonemapping",
                self.hdr.tonemapper.name().to_string(),
                Tonemapper::ALL.iter().map(|t| t.name().to_string()).collect(),
            ),
            Property::ranged(
                "exposure (EV)",
                self.hdr.exposure,
                HdrSettings::EXPOSURE_RANGE.0,
                HdrSettings::EXPOSURE_RANGE.1,
            ),
            Property::bool("auto exposure", self.hdr.auto_exposure),
            Property::ranged(
                "adaptation speed",
                self.hdr.adaptation_speed,
                HdrSettings::ADAPTATION_SPEED_RANGE.0,
                HdrSettings::ADAPTATION_SPEED_RANGE.1,
            ),
            Property::bool("bloom", self.hdr.bloom),
            Property::ranged(
                "bloom intensity",
                self.hdr.bloom_intensity,
                HdrSettings::BLOOM_INTENSITY_RANGE.0,
                HdrSettings::BLOOM_INTENSITY_RANGE.1,
            ),
            Property::ranged(
                "bloom threshold",
                self.hdr.bloom_threshold,
                HdrSettings::BLOOM_THRESHOLD_RANGE.0,
                HdrSettings::BLOOM_THRESHOLD_RANGE.1,
            ),
            Property::choice(
                "weather",
                self.weather.mode.name().to_string(),
                WeatherMode::ALL.iter().map(|m| m.name().to_string()).collect(),
            ),
            Property::ranged(
                "precipitation",
                self.weather.intensity,
                Weather::INTENSITY_RANGE.0,
                Weather::INTENSITY_RANGE.1,
            ),
            Property::ranged(
                "weather spell (s)",
                self.weather.spell_seconds,
                Weather::SPELL_RANGE.0,
                Weather::SPELL_RANGE.1,
            ),
            Property::ranged("wetness", self.weather.wetness, 0.0, 1.0),
            Property::ranged("puddles", self.weather.puddles, 0.0, 1.0),
            Property::ranged("snow cover", self.weather.snow_cover, 0.0, 1.0),
            Property::ranged(
                "snow depth (m)",
                self.weather.snow_depth,
                0.0,
                crate::core::renderer::weather::MAX_SNOW_DEPTH,
            ),
        ]
    }

    fn set_property(&mut self, name: &str, value: PropertyValue) {
        match (name, value) {
            ("time of day", PropertyValue::Float(v)) => self.set_time_of_day(v),
            ("day length (s)", PropertyValue::Float(v)) => self.set_day_length_seconds(v),
            ("paused", PropertyValue::Bool(v)) => self.paused = v,
            ("cloud coverage", PropertyValue::Float(v)) => self.set_cloud_coverage(v),
            ("cloud type", PropertyValue::Text(v)) => {
                if let Some(cloud_type) = CloudType::from_name(&v) {
                    self.clouds.cloud_type = cloud_type;
                }
            }
            ("altocumulus", PropertyValue::Float(v)) => self.clouds.set_altocumulus(v),
            ("cirrus", PropertyValue::Float(v)) => self.clouds.set_cirrus(v),
            ("cloud reprojection", PropertyValue::Bool(v)) => self.clouds.reprojection = v,
            ("staggered near shadows", PropertyValue::Bool(v)) => {
                self.shadows.stagger_second_cascade = v
            }
            ("moonlight", PropertyValue::Float(v)) => self.global_illumination.set_moonlight(v),
            ("night sky light", PropertyValue::Float(v)) => {
                self.global_illumination.set_night_sky(v)
            }
            ("light bounce", PropertyValue::Float(v)) => self.global_illumination.set_bounce(v),
            ("ambient occlusion", PropertyValue::Bool(v)) => self.ambient_occlusion.enabled = v,
            ("AO radius", PropertyValue::Float(v)) => self.ambient_occlusion.set_radius(v),
            ("AO intensity", PropertyValue::Float(v)) => self.ambient_occlusion.set_intensity(v),
            ("tonemapping", PropertyValue::Text(v)) => {
                if let Some(tonemapper) = Tonemapper::from_name(&v) {
                    self.hdr.tonemapper = tonemapper;
                }
            }
            ("exposure (EV)", PropertyValue::Float(v)) => self.hdr.set_exposure(v),
            ("auto exposure", PropertyValue::Bool(v)) => self.hdr.auto_exposure = v,
            ("adaptation speed", PropertyValue::Float(v)) => self.hdr.set_adaptation_speed(v),
            ("bloom", PropertyValue::Bool(v)) => self.hdr.bloom = v,
            ("bloom intensity", PropertyValue::Float(v)) => self.hdr.set_bloom_intensity(v),
            ("bloom threshold", PropertyValue::Float(v)) => self.hdr.set_bloom_threshold(v),
            ("weather", PropertyValue::Text(v)) => {
                if let Some(mode) = WeatherMode::from_name(&v) {
                    self.weather.mode = mode;
                }
            }
            ("precipitation", PropertyValue::Float(v)) => self.weather.set_intensity(v),
            ("weather spell (s)", PropertyValue::Float(v)) => self.weather.set_spell_seconds(v),
            ("wetness", PropertyValue::Float(v)) => self.weather.set_wetness(v),
            ("puddles", PropertyValue::Float(v)) => self.weather.set_puddles(v),
            ("snow cover", PropertyValue::Float(v)) => self.weather.set_snow_cover(v),
            ("snow depth (m)", PropertyValue::Float(v)) => self.weather.set_snow_depth(v),
            _ => {}
        }
    }
}

/// One cascade of the sun's shadow map.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowCascade {
    /// World to the cascade's clip space.
    pub view_projection: Matrix4<f32>,
    /// World size of one shadow map texel.
    pub texel_size: f32,
}

/// Far distance of each cascade: the "practical split scheme", a mix of
/// logarithmic splits (even detail per distance) and even ones.
fn cascade_splits(near: f32, far: f32) -> [f32; SHADOW_CASCADES] {
    std::array::from_fn(|i| {
        let t = (i + 1) as f32 / SHADOW_CASCADES as f32;
        let logarithmic = near * (far / near).powf(t);
        let even = near + (far - near) * t;
        SPLIT_LAMBDA * logarithmic + (1.0 - SPLIT_LAMBDA) * even
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cascades_split_the_shadow_distance_closer_near_the_camera() {
        let splits = cascade_splits(0.1, SHADOW_DISTANCE);
        assert!((splits[SHADOW_CASCADES - 1] - SHADOW_DISTANCE).abs() < 1e-3);
        for pair in splits.windows(2) {
            assert!(pair[0] < pair[1]);
        }
        // The first cascade covers far less than an even share.
        assert!(splits[0] < SHADOW_DISTANCE / SHADOW_CASCADES as f32);
    }

    #[test]
    fn shadows_are_cast_from_the_sun_by_day_and_the_moon_by_night() {
        let mut sky = SkyLight::new((30.0, 20.0, 10.0));
        sky.set_time_of_day(0.5);
        assert!((sky.shadow_light_direction() - sky.get_sun_direction()).magnitude() < 1e-6);
        sky.set_time_of_day(0.0);
        assert!((sky.shadow_light_direction() - sky.get_moon_direction()).magnitude() < 1e-6);
        assert!(sky.shadow_light_direction().y > 0.5);
    }

    #[test]
    fn the_second_cascade_is_staggered_only_when_asked_and_saved() {
        let mut sky = SkyLight::new((30.0, 20.0, 10.0));
        assert_eq!(sky.shadows().intervals(), [1, 1, 2, 4]);
        sky.set_property("staggered near shadows", PropertyValue::Bool(true));
        assert_eq!(sky.shadows().intervals(), [1, 2, 2, 4]);
        match sky.to_data() {
            Some(ComponentData::SkyLight { shadows, .. }) => assert!(shadows.stagger_second_cascade),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn global_illumination_is_edited_and_saved_with_the_sky() {
        let mut sky = SkyLight::new((30.0, 20.0, 10.0));
        sky.set_property("moonlight", PropertyValue::Float(2.0));
        sky.set_property("night sky light", PropertyValue::Float(99.0));
        sky.set_property("light bounce", PropertyValue::Float(0.5));
        let gi = *sky.global_illumination();
        assert_eq!(gi.moonlight, 2.0);
        assert_eq!(gi.night_sky, GlobalIllumination::NIGHT_SKY_RANGE.1);
        assert_eq!(gi.bounce, 0.5);
        match sky.to_data() {
            Some(ComponentData::SkyLight { global_illumination, .. }) => {
                assert_eq!(global_illumination, gi)
            }
            other => panic!("{other:?}"),
        }
    }
}
