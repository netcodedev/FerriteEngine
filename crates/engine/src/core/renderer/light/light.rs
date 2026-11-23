//! Lights placed in the scene: point lights (a lantern, a torch), spot
//! lights (a street lamp, a flashlight) and extra directional lights (a
//! coloured fill), each in any colour.
//!
//! The `Light` component only describes a light. Every frame the scene
//! gathers the lights with their entity's world transform (`gather`),
//! keeps the ones that matter most for the camera (`select`), and uploads
//! them into one uniform buffer (`LightBuffer`) that every lit shader reads
//! in `pbr/lighting.glsl` (`localLights`, `localBounce`).

use cgmath::{InnerSpace, Matrix4, Point3, Vector3, Vector4};
use glfw::{Glfw, WindowEvent};
use serde::{Deserialize, Serialize};

use crate::core::{
    entity::{
        component::{Component, Property, PropertyValue},
        Entity,
    },
    gfx,
    scene::{scene_file::ComponentData, Scene},
};

/// Most lights a frame shades with; must match `MAX_LIGHTS` in
/// `pbr/lighting.glsl`.
pub const MAX_LIGHTS: usize = 64;
/// Uniform buffer binding point of the lights; must match the `binding` of
/// the `Lights` block in `pbr/lighting.glsl`.
pub const LIGHTS_BINDING: u32 = 1;

/// How a light shines.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LightKind {
    /// From one point in every direction, fading out at its range.
    #[default]
    Point,
    /// From one point in a cone along the entity's −Y axis.
    Spot,
    /// Parallel light along the entity's −Y axis everywhere, like a second
    /// sun without shadows.
    Directional,
}

impl LightKind {
    pub const ALL: [LightKind; 3] = [LightKind::Point, LightKind::Spot, LightKind::Directional];

    pub fn name(self) -> &'static str {
        match self {
            LightKind::Point => "Point",
            LightKind::Spot => "Spot",
            LightKind::Directional => "Directional",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|k| k.name().eq_ignore_ascii_case(name))
    }

    /// Value of `kind` in the shader's light struct.
    fn shader_value(self) -> f32 {
        match self {
            LightKind::Point => 0.0,
            LightKind::Spot => 1.0,
            LightKind::Directional => 2.0,
        }
    }
}

/// What a light is, as saved in a scene file. Colours are sRGB, 0 to 1,
/// the way a colour picker shows them.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(default)]
pub struct LightSettings {
    pub kind: LightKind,
    pub color: [f32; 3],
    /// Brightness: what a surface one metre away facing the light gets.
    /// The midday sun is about 3.5 on this scale.
    pub intensity: f32,
    /// Distance in metres at which a point or spot light has faded to
    /// nothing. Directional lights reach everywhere.
    pub range: f32,
    /// Spot lights: half angle of the cone's fully lit core, in degrees.
    pub inner_angle: f32,
    /// Spot lights: half angle of the cone's edge, in degrees.
    pub outer_angle: f32,
    /// How much of its light bounces off the surroundings onto everything
    /// nearby, around corners too, times the sky light's "light bounce".
    /// 0 lights only what it shines on directly.
    pub bounce: f32,
    pub enabled: bool,
    /// Point and spot lights: whether things block it. With cube maps
    /// (`shadow`) only the few lights nearest the camera get shadows each
    /// frame; ray traced (`shadows.raytraced_lights`) all of them.
    pub shadows: bool,
    /// Point and spot lights: the lamp's size, in metres (a sphere this
    /// big). Ray traced shadows (`shadows.raytraced_lights`) soften with
    /// it; the shadow maps ignore it.
    pub radius: f32,
    /// How much its brightness wavers, like a flame: 0 steady, 1 a lot.
    pub flicker: f32,
}

impl Default for LightSettings {
    fn default() -> Self {
        Self {
            kind: LightKind::Point,
            // A warm, fire-like light.
            color: [1.0, 0.72, 0.42],
            intensity: 8.0,
            range: 12.0,
            inner_angle: 20.0,
            outer_angle: 30.0,
            bounce: 1.0,
            enabled: true,
            shadows: true,
            radius: 0.05,
            flicker: 0.0,
        }
    }
}

impl LightSettings {
    /// What a light may be given: up to car headlights' beams, which
    /// have to reach far.
    pub const INTENSITY_RANGE: (f32, f32) = (0.0, 20_000.0);
    /// What the inspector offers (lamps, torches, floodlights); stronger
    /// lights are set from scripts.
    pub const INTENSITY_SLIDER: (f32, f32) = (0.0, 200.0);
    pub const RANGE_RANGE: (f32, f32) = (0.1, 200.0);
    pub const ANGLE_RANGE: (f32, f32) = (1.0, 89.0);
    pub const BOUNCE_RANGE: (f32, f32) = (0.0, 4.0);
    pub const RADIUS_RANGE: (f32, f32) = (0.0, 0.5);

    pub fn set_color(&mut self, color: [f32; 3]) {
        self.color = color.map(|c| if c.is_finite() { c.clamp(0.0, 1.0) } else { 1.0 });
    }

    pub fn set_intensity(&mut self, intensity: f32) {
        self.intensity = clamp_finite(intensity, Self::INTENSITY_RANGE, 8.0);
    }

    pub fn set_range(&mut self, range: f32) {
        self.range = clamp_finite(range, Self::RANGE_RANGE, 12.0);
    }

    /// Set the core's half angle; the edge widens to stay outside it.
    pub fn set_inner_angle(&mut self, degrees: f32) {
        self.inner_angle = clamp_finite(degrees, Self::ANGLE_RANGE, 20.0);
        self.outer_angle = self.outer_angle.max(self.inner_angle);
    }

    /// Set the edge's half angle; the core narrows to stay inside it.
    pub fn set_outer_angle(&mut self, degrees: f32) {
        self.outer_angle = clamp_finite(degrees, Self::ANGLE_RANGE, 30.0);
        self.inner_angle = self.inner_angle.min(self.outer_angle);
    }

    pub fn set_bounce(&mut self, bounce: f32) {
        self.bounce = clamp_finite(bounce, Self::BOUNCE_RANGE, 1.0);
    }

    pub fn set_radius(&mut self, radius: f32) {
        self.radius = clamp_finite(radius, Self::RADIUS_RANGE, 0.05);
    }

    /// Every field through its setter, so out of range values (from a hand
    /// edited scene file) are clamped.
    pub fn sanitized(self) -> Self {
        let mut s = Self {
            kind: self.kind,
            enabled: self.enabled,
            shadows: self.shadows,
            // Set below, in an order that keeps inner ≤ outer.
            inner_angle: Self::ANGLE_RANGE.0,
            outer_angle: Self::ANGLE_RANGE.1,
            ..Self::default()
        };
        s.set_color(self.color);
        s.set_intensity(self.intensity);
        s.set_range(self.range);
        s.set_outer_angle(self.outer_angle);
        s.set_inner_angle(self.inner_angle.min(s.outer_angle));
        s.set_bounce(self.bounce);
        s.set_radius(self.radius);
        s.flicker = clamp_finite(self.flicker, (0.0, 1.0), 0.0);
        s
    }

    /// The colour in linear light, times the intensity: what the shader
    /// multiplies the BRDF with.
    pub fn radiance(&self) -> [f32; 3] {
        self.color.map(|c| srgb_to_linear(c) * self.intensity)
    }
}

/// `value` clamped to `min..=max`, or `fallback` when it is NaN or
/// infinite (`f32::clamp` lets NaN through).
pub(crate) fn clamp_finite(value: f32, (min, max): (f32, f32), fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}

/// sRGB transfer function, 0..1 to linear 0..1.
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// A light on an entity. Where it is and which way it points come from the
/// entity: spot and directional lights shine along its −Y axis, straight
/// down while it is not rotated.
#[derive(Clone, Debug, Default)]
pub struct Light {
    settings: LightSettings,
    /// Seconds it has been running, for the flicker.
    time: f32,
    /// Where the flicker starts, so lights do not flicker in step.
    phase: f32,
}

/// How bright a flickering light is at `time`, 0..1 of its intensity for
/// `flicker` 0..1: a few waves of different speeds, like a flame.
pub fn flicker_at(flicker: f32, time: f32) -> f32 {
    let t = time;
    let wave = (t * 7.3).sin() * 0.45 + (t * 13.1 + 1.7).sin() * 0.3 + (t * 23.7 + 4.1).sin() * 0.25;
    // wave is -1..1; at most `flicker` of the light goes.
    1.0 - flicker * 0.5 * (1.0 + wave)
}

impl Light {
    pub fn new(settings: LightSettings) -> Self {
        static PHASE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = PHASE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self {
            settings: settings.sanitized(),
            time: 0.0,
            phase: n as f32 * 3.17,
        }
    }

    pub fn point(color: [f32; 3], intensity: f32, range: f32) -> Self {
        Self::new(LightSettings {
            kind: LightKind::Point,
            color,
            intensity,
            range,
            ..LightSettings::default()
        })
    }

    pub fn settings(&self) -> &LightSettings {
        &self.settings
    }

    pub fn set_settings(&mut self, settings: LightSettings) {
        self.settings = settings.sanitized();
    }

    /// The light in the world, for an entity with `transform` (its world
    /// transform). `None` while it is switched off or gives no light.
    pub fn source(&self, transform: &Matrix4<f32>) -> Option<LightSource> {
        let s = &self.settings;
        if !s.enabled || s.intensity <= 0.0 {
            return None;
        }
        let position = Point3::from_homogeneous(transform * Vector4::new(0.0, 0.0, 0.0, 1.0));
        let down = (transform * Vector4::new(0.0, -1.0, 0.0, 0.0)).truncate();
        let direction = if down.magnitude2() > 1e-12 {
            down.normalize()
        } else {
            -Vector3::unit_y()
        };
        Some(LightSource {
            kind: s.kind,
            position,
            direction,
            radiance: s
                .radiance()
                .map(|c| c * flicker_at(s.flicker, self.time + self.phase)),
            range: s.range,
            cos_inner: s.inner_angle.to_radians().cos(),
            cos_outer: s.outer_angle.to_radians().cos(),
            bounce: s.bounce,
            casts_shadows: s.shadows && s.kind != LightKind::Directional,
            shadow_layer: None,
            radius: s.radius,
        })
    }
}

impl Component for Light {
    fn update(&mut self, _: &mut Scene, _: &mut Entity, delta_time: f64) {
        self.time += delta_time as f32;
    }

    fn update_in_editor(&mut self, _: &mut Scene, _: &mut Entity, delta_time: f64) {
        self.time += delta_time as f32;
    }

    fn handle_event(&mut self, _: &mut Glfw, _: &mut glfw::Window, _: &WindowEvent) {}

    fn display_name(&self) -> String {
        "Light".into()
    }

    fn to_data(&self) -> Option<ComponentData> {
        Some(ComponentData::light(self.settings))
    }

    fn properties(&self) -> Vec<Property> {
        let s = &self.settings;
        let [r, g, b] = s.color;
        vec![
            Property::choice(
                "kind",
                s.kind.name().to_string(),
                LightKind::ALL.iter().map(|k| k.name().to_string()).collect(),
            ),
            Property::bool("enabled", s.enabled),
            Property::bool("shadows", s.shadows),
            Property::ranged("flicker", s.flicker, 0.0, 1.0),
            Property::ranged("color r", r, 0.0, 1.0),
            Property::ranged("color g", g, 0.0, 1.0),
            Property::ranged("color b", b, 0.0, 1.0),
            Property::ranged(
                "intensity",
                s.intensity,
                LightSettings::INTENSITY_SLIDER.0,
                LightSettings::INTENSITY_SLIDER.1.max(s.intensity),
            ),
            Property::ranged(
                "range",
                s.range,
                LightSettings::RANGE_RANGE.0,
                LightSettings::RANGE_RANGE.1,
            ),
            Property::ranged(
                "inner angle",
                s.inner_angle,
                LightSettings::ANGLE_RANGE.0,
                LightSettings::ANGLE_RANGE.1,
            ),
            Property::ranged(
                "outer angle",
                s.outer_angle,
                LightSettings::ANGLE_RANGE.0,
                LightSettings::ANGLE_RANGE.1,
            ),
            Property::ranged(
                "bounce",
                s.bounce,
                LightSettings::BOUNCE_RANGE.0,
                LightSettings::BOUNCE_RANGE.1,
            ),
            Property::ranged(
                "radius",
                s.radius,
                LightSettings::RADIUS_RANGE.0,
                LightSettings::RADIUS_RANGE.1,
            ),
        ]
    }

    fn set_property(&mut self, name: &str, value: PropertyValue) {
        let s = &mut self.settings;
        let float = value.as_f32();
        match name {
            "kind" => {
                if let Some(kind) = value.as_text().and_then(LightKind::from_name) {
                    s.kind = kind;
                }
            }
            "enabled" => {
                if let Some(v) = value.as_bool() {
                    s.enabled = v;
                }
            }
            "shadows" => {
                if let Some(v) = value.as_bool() {
                    s.shadows = v;
                }
            }
            "color r" | "color g" | "color b" => {
                if let Some(v) = float {
                    let i = match name {
                        "color r" => 0,
                        "color g" => 1,
                        _ => 2,
                    };
                    let mut color = s.color;
                    color[i] = v;
                    s.set_color(color);
                }
            }
            "intensity" => float.into_iter().for_each(|v| s.set_intensity(v)),
            "range" => float.into_iter().for_each(|v| s.set_range(v)),
            "inner angle" => float.into_iter().for_each(|v| s.set_inner_angle(v)),
            "outer angle" => float.into_iter().for_each(|v| s.set_outer_angle(v)),
            "bounce" => float.into_iter().for_each(|v| s.set_bounce(v)),
            "radius" => float.into_iter().for_each(|v| s.set_radius(v)),
            "flicker" => float.into_iter().for_each(|v| s.flicker = v.clamp(0.0, 1.0)),
            _ => {}
        }
    }
}

/// A light where it is in the world this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightSource {
    pub kind: LightKind,
    pub position: Point3<f32>,
    /// Which way spot and directional lights shine (unit length).
    pub direction: Vector3<f32>,
    /// Linear colour times intensity.
    pub radiance: [f32; 3],
    pub range: f32,
    pub cos_inner: f32,
    pub cos_outer: f32,
    pub bounce: f32,
    /// Whether it should get a shadow map (`shadow::assign` decides).
    pub casts_shadows: bool,
    /// Its layer in the light shadow maps this frame, if it got one.
    pub shadow_layer: Option<u32>,
    /// The lamp's size (metres), for soft ray traced shadows.
    pub radius: f32,
}

impl LightSource {
    /// How much it matters to a camera at `eye`, higher first: directional
    /// lights always, then by how far the camera is from the light's reach,
    /// brighter lights winning ties.
    pub(super) fn priority(&self, eye: Point3<f32>) -> f32 {
        match self.kind {
            LightKind::Directional => f32::INFINITY,
            _ => {
                let outside = ((self.position - eye).magnitude() - self.range).max(0.0);
                let brightness = self.radiance.iter().sum::<f32>();
                -outside + brightness.min(1000.0) * 1e-4
            }
        }
    }

    /// The light as four `vec4`s, laid out like `struct Light` in
    /// `pbr/lighting.glsl` (std140).
    pub fn packed(&self) -> [[f32; 4]; 4] {
        let [r, g, b] = self.radiance;
        let p = self.position;
        let d = self.direction;
        [
            [p.x, p.y, p.z, self.range],
            [r, g, b, self.bounce],
            [d.x, d.y, d.z, self.kind.shader_value()],
            [
                self.cos_inner,
                self.cos_outer,
                self.shadow_layer.map_or(-1.0, |layer| layer as f32),
                // Read by `castsShadows` in `pbr/lights.glsl`.
                if self.casts_shadows && self.kind != LightKind::Directional {
                    self.radius.max(0.0)
                } else {
                    -1.0
                },
            ],
        ]
    }
}

/// Every enabled light in the scene with its entity's world transform, and
/// the lights components give themselves (see `Component::lights`) for a
/// camera at `eye`.
pub fn gather(scene: &Scene, eye: Point3<f32>) -> Vec<LightSource> {
    fn visit(entity: &Entity, parent: &Matrix4<f32>, eye: Point3<f32>, out: &mut Vec<LightSource>) {
        let transform = parent * entity.local_transform();
        for component in entity.components() {
            if let Some(light) = component.as_any().downcast_ref::<Light>() {
                out.extend(light.source(&transform));
            }
            component.lights(&transform, eye, out);
        }
        for child in entity.get_children() {
            visit(child, &transform, eye, out);
        }
    }
    let mut out = Vec::new();
    let identity = Matrix4::from_scale(1.0);
    for entity in scene.get_entities() {
        visit(entity, &identity, eye, &mut out);
    }
    out
}

/// The lights a camera at `eye` gets, at most `MAX_LIGHTS`: lights whose
/// reach the camera is in or nearest first.
pub fn select(mut lights: Vec<LightSource>, eye: Point3<f32>) -> Vec<LightSource> {
    if lights.len() > MAX_LIGHTS {
        lights.sort_by(|a, b| b.priority(eye).total_cmp(&a.priority(eye)));
        lights.truncate(MAX_LIGHTS);
    }
    lights
}

/// The lights uploaded for the shaders: a uniform buffer bound to
/// `LIGHTS_BINDING`.
pub struct LightBuffer {
    buffer: gfx::Buffer,
}

impl LightBuffer {
    /// std140 size of the `Lights` block: four `vec4` per light.
    const SIZE: usize = MAX_LIGHTS * 4 * 4 * std::mem::size_of::<f32>();

    pub fn new() -> Self {
        Self {
            buffer: gfx::Buffer::new(gfx::BufferUsage::UNIFORM, gfx::BufferMemory::Dynamic, Self::SIZE),
        }
    }

    /// Upload `lights` (at most `MAX_LIGHTS` are used) and bind the buffer.
    pub fn upload(&mut self, lights: &[LightSource]) {
        // The whole block, lights past the count zero.
        let mut data = vec![[[0.0f32; 4]; 4]; MAX_LIGHTS];
        for (slot, light) in data.iter_mut().zip(lights) {
            *slot = light.packed();
        }
        self.buffer.replace(gfx::as_bytes(&data));
        gfx::bind_uniform_buffer(LIGHTS_BINDING, &self.buffer);
    }
}

#[cfg(test)]
mod tests {
    use cgmath::{Deg, EuclideanSpace, Quaternion, Rotation3};

    use super::*;

    fn at(x: f32, y: f32, z: f32) -> Matrix4<f32> {
        Matrix4::from_translation(Vector3::new(x, y, z))
    }

    #[test]
    fn a_light_is_where_its_entity_is_and_shines_down_its_minus_y_axis() {
        let light = Light::new(LightSettings {
            kind: LightKind::Spot,
            ..LightSettings::default()
        });
        let source = light.source(&at(1.0, 2.0, 3.0)).unwrap();
        assert_eq!(source.position, Point3::new(1.0, 2.0, 3.0));
        assert!((source.direction - Vector3::new(0.0, -1.0, 0.0)).magnitude() < 1e-6);

        // Tipped 90° about Z, −Y turns to +X.
        let turned = at(0.0, 0.0, 0.0) * Matrix4::from(Quaternion::from_angle_z(Deg(90.0)));
        let source = light.source(&turned).unwrap();
        assert!((source.direction - Vector3::new(1.0, 0.0, 0.0)).magnitude() < 1e-5);
    }

    #[test]
    fn a_scaled_entity_does_not_change_the_lights_direction_length() {
        let light = Light::default();
        let source = light.source(&Matrix4::from_scale(3.0)).unwrap();
        assert!((source.direction.magnitude() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn switched_off_and_black_lights_give_no_source() {
        let mut light = Light::default();
        light.set_property("enabled", PropertyValue::Bool(false));
        assert!(light.source(&at(0.0, 0.0, 0.0)).is_none());
        let dark = Light::new(LightSettings {
            intensity: 0.0,
            ..LightSettings::default()
        });
        assert!(dark.source(&at(0.0, 0.0, 0.0)).is_none());
    }

    #[test]
    fn colours_are_srgb_and_reach_the_shader_linear_times_intensity() {
        let light = Light::point([1.0, 0.5, 0.0], 4.0, 10.0);
        let [r, g, b] = light.settings().radiance();
        assert!((r - 4.0).abs() < 1e-5);
        assert!((g - 4.0 * 0.214).abs() < 0.01, "{g}");
        assert_eq!(b, 0.0);
        assert!((srgb_to_linear(0.0)).abs() < 1e-7);
        assert!((srgb_to_linear(1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn settings_are_clamped_and_cone_angles_stay_ordered() {
        let s = LightSettings {
            color: [2.0, -1.0, f32::NAN],
            intensity: -5.0,
            range: 1e9,
            inner_angle: 70.0,
            outer_angle: 10.0,
            bounce: 99.0,
            ..LightSettings::default()
        }
        .sanitized();
        assert_eq!(s.color, [1.0, 0.0, 1.0]);
        assert_eq!(s.intensity, 0.0);
        assert_eq!(s.range, LightSettings::RANGE_RANGE.1);
        assert!(s.inner_angle <= s.outer_angle);
        assert_eq!(s.bounce, LightSettings::BOUNCE_RANGE.1);

        let mut s = LightSettings::default();
        s.set_inner_angle(50.0);
        assert_eq!((s.inner_angle, s.outer_angle), (50.0, 50.0));
        s.set_outer_angle(5.0);
        assert_eq!((s.inner_angle, s.outer_angle), (5.0, 5.0));
    }

    #[test]
    fn properties_round_trip_through_set_property() {
        let mut light = Light::default();
        light.set_property("kind", PropertyValue::Text("spot".into()));
        light.set_property("color g", PropertyValue::Float(0.25));
        light.set_property("intensity", PropertyValue::Int(20));
        light.set_property("range", PropertyValue::Float(30.0));
        light.set_property("outer angle", PropertyValue::Float(40.0));
        light.set_property("bounce", PropertyValue::Float(0.5));
        let s = light.settings();
        assert_eq!(s.kind, LightKind::Spot);
        assert_eq!(s.color[1], 0.25);
        assert_eq!((s.intensity, s.range, s.outer_angle, s.bounce), (20.0, 30.0, 40.0, 0.5));
        // Every property it lists is one it takes back.
        for p in light.properties() {
            let mut copy = light.clone();
            copy.set_property(&p.name, p.value.clone());
            assert_eq!(copy.settings(), light.settings(), "{}", p.name);
        }
    }

    #[test]
    fn it_saves_as_its_settings() {
        let light = Light::point([0.2, 0.4, 1.0], 3.0, 6.0);
        assert_eq!(light.to_data(), Some(ComponentData::light(*light.settings())));
    }

    #[test]
    fn packing_matches_the_shaders_struct() {
        let light = Light::new(LightSettings {
            kind: LightKind::Spot,
            color: [1.0, 1.0, 1.0],
            intensity: 2.0,
            range: 9.0,
            inner_angle: 60.0,
            outer_angle: 60.0,
            bounce: 0.5,
            enabled: true,
            shadows: true,
            radius: 0.2,
            flicker: 0.0,
        });
        let p = light.source(&at(1.0, 2.0, 3.0)).unwrap().packed();
        assert_eq!(p[0], [1.0, 2.0, 3.0, 9.0]);
        assert_eq!(p[1], [2.0, 2.0, 2.0, 0.5]);
        assert_eq!(p[2][3], 1.0);
        assert!((p[3][0] - 0.5).abs() < 1e-6 && (p[3][1] - 0.5).abs() < 1e-6);
        // Its radius, read by `castsShadows` in `pbr/lights.glsl`.
        assert_eq!(p[3][3], 0.2);
    }

    #[test]
    fn lights_without_ray_traced_shadows_pack_a_negative_radius() {
        let mut settings = LightSettings { radius: 0.3, ..LightSettings::default() };
        settings.shadows = false;
        let unshadowed = Light::new(settings).source(&at(0.0, 0.0, 0.0)).unwrap().packed();
        assert_eq!(unshadowed[3][3], -1.0);
        settings.shadows = true;
        settings.kind = LightKind::Directional;
        let sun = Light::new(settings).source(&at(0.0, 0.0, 0.0)).unwrap().packed();
        assert_eq!(sun[3][3], -1.0);
    }

    #[test]
    fn the_radius_is_clamped_and_set_as_a_property() {
        let s = LightSettings { radius: 7.0, ..LightSettings::default() }.sanitized();
        assert_eq!(s.radius, LightSettings::RADIUS_RANGE.1);
        let s = LightSettings { radius: f32::NAN, ..LightSettings::default() }.sanitized();
        assert_eq!(s.radius, LightSettings::default().radius);
        let mut light = Light::default();
        light.set_property("radius", PropertyValue::Float(0.25));
        assert_eq!(light.settings().radius, 0.25);
    }

    #[test]
    fn with_too_many_lights_the_ones_near_the_camera_are_kept() {
        let eye = Point3::origin();
        let mut lights: Vec<LightSource> = (0..MAX_LIGHTS + 10)
            .map(|i| {
                Light::point([1.0; 3], 1.0, 5.0)
                    .source(&at(100.0 + i as f32 * 10.0, 0.0, 0.0))
                    .unwrap()
            })
            .collect();
        let near = Light::point([1.0; 3], 1.0, 5.0).source(&at(3.0, 0.0, 0.0)).unwrap();
        let sun = Light::new(LightSettings {
            kind: LightKind::Directional,
            ..LightSettings::default()
        })
        .source(&at(5000.0, 0.0, 0.0))
        .unwrap();
        lights.push(near);
        lights.push(sun);
        let kept = select(lights, eye);
        assert_eq!(kept.len(), MAX_LIGHTS);
        assert!(kept.contains(&near));
        assert!(kept.contains(&sun));
        let farthest = Light::point([1.0; 3], 1.0, 5.0)
            .source(&at(100.0 + (MAX_LIGHTS + 9) as f32 * 10.0, 0.0, 0.0))
            .unwrap();
        assert!(!kept.contains(&farthest));
    }

    #[test]
    fn a_few_lights_are_kept_in_their_order() {
        let a = Light::point([1.0; 3], 1.0, 5.0).source(&at(50.0, 0.0, 0.0)).unwrap();
        let b = Light::point([1.0; 3], 1.0, 5.0).source(&at(1.0, 0.0, 0.0)).unwrap();
        assert_eq!(select(vec![a, b], Point3::origin()), vec![a, b]);
    }

    #[test]
    fn a_flickering_light_wavers_within_its_flicker() {
        let mut lowest: f32 = 1.0;
        for i in 0..1000 {
            let f = flicker_at(0.6, i as f32 * 0.013);
            assert!((0.4 - 1e-4..=1.0 + 1e-4).contains(&f), "{f}");
            lowest = lowest.min(f);
        }
        assert!(lowest < 0.6);
        assert_eq!(flicker_at(0.0, 3.0), 1.0);
    }

    #[test]
    fn light_kinds_have_names_for_the_inspector() {
        for kind in LightKind::ALL {
            assert_eq!(LightKind::from_name(kind.name()), Some(kind));
        }
        assert_eq!(LightKind::from_name("DIRECTIONAL"), Some(LightKind::Directional));
        assert_eq!(LightKind::from_name("area"), None);
    }
}
