//! Physically based shading of model materials: the light a scene gives a
//! PBR shader, and binding a material's maps.

use std::collections::HashMap;

use cgmath::{EuclideanSpace, InnerSpace, Point3, Vector3};
use russimp_ng::material::TextureType;

use crate::core::{
    renderer::{
        light::{
            global_illumination::IndirectShading,
            shadow::LIGHT_SHADOW_UNIT,
            skylight::{ShadowCascade, SkyLight, SHADOW_CASCADES},
        },
        hdr,
        shader::Shader,
        ssao::SSAO_UNIT,
        texture::Texture,
        weather::{WeatherMaps, WeatherShading},
    },
    scene::Scene,
};

/// Sun, moon, sky, the scene's lights, shadows, the BRDF (`shadeSurface`, `toDisplay`) and the
/// weather on surfaces (`applyWeather`). Goes
/// right after a fragment shader's `#version` line.
pub const LIGHTING_GLSL: &str = concat!(
    include_str!("../hdr/tonemap.glsl"),
    include_str!("lighting.glsl"),
    include_str!("../weather/weather_maps.glsl"),
    include_str!("../weather/weather.glsl")
);
pub const MATERIAL_GLSL: &str = include_str!("material.glsl");

/// Texture unit the scene binds the sun's shadow map to.
const SHADOW_MAP_UNIT: i32 = 15;

/// A model fragment shader: the shared lighting and material code
/// (`shadePbr`) put in front of `body`.
pub fn fragment_source(body: &str) -> String {
    format!("#version 460 core\n{LIGHTING_GLSL}\n{MATERIAL_GLSL}\n{body}")
}

thread_local! {
    /// The lighting each program was last given, by program and pass kind.
    static APPLIED: std::cell::RefCell<Vec<(u64, (bool, bool, Lighting))>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The light a PBR shader is drawn with.
#[derive(Clone, Copy, PartialEq)]
pub struct Lighting {
    /// Toward the sun.
    pub sun_direction: Vector3<f32>,
    pub camera_position: Point3<f32>,
    pub cloud_coverage: f32,
    /// The sun's shadow cascades, when the scene has a shadow map.
    pub shadow_cascades: Option<[ShadowCascade; SHADOW_CASCADES]>,
    /// Window position of the scene's ambient occlusion (bound to
    /// `SSAO_UNIT`) while it shades the camera's view.
    pub ambient_occlusion_origin: Option<[i32; 2]>,
    /// Rain and snow on surfaces.
    pub weather: WeatherShading,
    /// Where the weather's maps lie (bound to `weather::EXPOSURE_UNIT` and
    /// `weather::SNOW_TRAIL_UNIT`) while the scene is shaded.
    pub weather_maps: Option<WeatherMaps>,
    /// The moon, the night sky and lamp light bounce.
    pub indirect: IndirectShading,
    /// How many of the scene's lights the light buffer holds this frame
    /// (`light::light::LightBuffer`); 0 shades without them.
    pub light_count: usize,
}

impl Lighting {
    /// The scene's sun, sky and camera. `None` without a sky light.
    pub fn from_scene(scene: &Scene) -> Option<Self> {
        // While the scene is drawn, the sun, sky and camera stay put: only
        // what the passes change is read again.
        if let Some(mut lighting) = scene.frame_lighting() {
            lighting.ambient_occlusion_origin = scene.ambient_occlusion_origin();
            lighting.weather_maps = scene.weather_maps();
            lighting.light_count = scene.light_count();
            return Some(lighting);
        }
        let skylight = scene.get_component::<SkyLight>()?;
        let camera_position = scene
            .get_active_camera()
            .map(|camera| camera.get_camera().get_eye_position())
            .unwrap_or_else(Point3::origin);
        Some(Self {
            sun_direction: skylight.get_sun_direction().normalize(),
            camera_position,
            cloud_coverage: skylight.effective_cloud_coverage(),
            shadow_cascades: scene
                .get_shadow_map_size()
                .map(|_| *skylight.shadow_cascades()),
            ambient_occlusion_origin: scene.ambient_occlusion_origin(),
            weather: skylight.weather().shading(),
            weather_maps: scene.weather_maps(),
            indirect: skylight.indirect_shading(),
            light_count: scene.light_count(),
        })
    }

    /// Clear daylight from `sun_direction` without shadows, for previews.
    pub fn preview(sun_direction: Vector3<f32>, camera_position: Point3<f32>) -> Self {
        Self {
            sun_direction: sun_direction.normalize(),
            camera_position,
            cloud_coverage: 0.0,
            shadow_cascades: None,
            ambient_occlusion_origin: None,
            weather: WeatherShading::default(),
            weather_maps: None,
            indirect: IndirectShading::preview(sun_direction.normalize()),
            light_count: 0,
        }
    }

    /// Set the lighting uniforms of the bound `shader`.
    pub fn apply(&self, shader: &Shader) {
        // The uniforms stay set on the program: the same lighting in the
        // same kind of pass needs no setting again (dozens of uniforms per
        // drawn model otherwise).
        // Per program, with the kind of pass: one program drawn in two
        // kinds of pass (the scene's view and an editor picture) must get
        // its uniforms set again on every switch, not keep the other's.
        let program = shader.program().serial();
        let wanted = (hdr::draws_to_view(), hdr::writes_linear(), *self);
        let unchanged = APPLIED.with(|applied| {
            let mut applied = applied.borrow_mut();
            match applied.iter_mut().find(|(k, _)| *k == program) {
                Some((_, last)) if *last == wanted => true,
                Some((_, last)) => {
                    *last = wanted;
                    false
                }
                None => {
                    // Programs come and go (script reloads): keep a bounded list.
                    if applied.len() >= 64 {
                        applied.remove(0);
                    }
                    applied.push((program, wanted));
                    false
                }
            }
        });
        if unchanged {
            return;
        }
        // Shadow maps, depth captures and the selection mask are drawn into
        // framebuffers of their own; the camera's view into the HDR target
        // (linear light) or, for previews, straight into the window.
        shader.set_uniform_1i("isShadowPass", !hdr::draws_to_view() as i32);
        shader.set_uniform_1i("hdrOutput", hdr::writes_linear() as i32);
        shader.set_uniform_3fv("sunDirection", &self.sun_direction);
        let eye = self.camera_position;
        shader.set_uniform_3f("cameraPosition", eye.x, eye.y, eye.z);
        shader.set_uniform_1f("cloudCoverage", self.cloud_coverage);
        shader.set_uniform_1i("shadowMap", SHADOW_MAP_UNIT);
        match &self.shadow_cascades {
            Some(cascades) => {
                let projections = cascades.map(|cascade| cascade.view_projection);
                shader.set_uniform_mat4_array("lightProjections", &projections);
                let texels = cascades.map(|cascade| cascade.texel_size);
                shader.set_uniform_1fv("cascadeTexel", &texels);
                // The graphics settings may leave far cascades out.
                let in_use = cascades.iter().take_while(|cascade| cascade.texel_size > 0.0).count();
                shader.set_uniform_1i("shadowCascadeCount", in_use as i32);
                shader.set_uniform_1i("useShadowMap", 1);
            }
            None => shader.set_uniform_1i("useShadowMap", 0),
        }
        // Set even when unused: a sampler left on unit 0 could clash with a
        // texture array bound there.
        shader.set_uniform_1i("ssaoMap", SSAO_UNIT as i32);
        match self.ambient_occlusion_origin {
            Some([x, y]) => {
                shader.set_uniform_1i("useSsao", 1);
                shader.set_uniform_2i("ssaoOrigin", x, y);
            }
            None => shader.set_uniform_1i("useSsao", 0),
        }
        let indirect = &self.indirect;
        shader.set_uniform_3fv("moonDirection", &indirect.moon_direction);
        shader.set_uniform_1f("moonlight", indirect.settings.moonlight);
        shader.set_uniform_1f("nightSky", indirect.settings.night_sky);
        shader.set_uniform_1f("lightBounce", indirect.settings.bounce);
        shader.set_uniform_1i("moonShadows", indirect.moon_shadows as i32);
        shader.set_uniform_1i("lightCount", self.light_count as i32);
        // Set even without shadows, like `ssaoMap`: left on unit 0 it would
        // clash with the material's maps.
        shader.set_uniform_1i("lightShadowMaps", LIGHT_SHADOW_UNIT as i32);
        self.weather.apply(shader, self.weather_maps.as_ref());
    }
}

/// Bind a material's maps to the bound `shader`. Maps the material lacks
/// are switched off: plain white, the surface normal, a default roughness,
/// no metal, no occlusion.
pub fn bind_material(shader: &Shader, textures: Option<&HashMap<TextureType, Texture>>) {
    let find = |types: &[TextureType]| {
        textures.and_then(|textures| types.iter().find_map(|t| textures.get(t)))
    };
    // A material may tag its colour map oddly; any map that is not one of
    // the data maps is taken for it.
    let base_color = find(&[TextureType::Diffuse, TextureType::BaseColor]).or_else(|| {
        textures.and_then(|textures| {
            textures.iter().find_map(|(t, texture)| (!is_data_map(t)).then_some(texture))
        })
    });
    bind(shader, 0, "baseColorMap", "hasBaseColorMap", base_color);
    bind(
        shader,
        1,
        "normalMap",
        "hasNormalMap",
        find(&[TextureType::Normals, TextureType::NormalCamera]),
    );
    bind(
        shader,
        2,
        "roughnessMap",
        "hasRoughnessMap",
        find(&[TextureType::Roughness]),
    );
    bind(shader, 3, "metallicMap", "hasMetallicMap", find(&[TextureType::Metalness]));
    bind(
        shader,
        4,
        "occlusionMap",
        "hasOcclusionMap",
        find(&[TextureType::AmbientOcclusion]),
    );
}

fn is_data_map(texture_type: &TextureType) -> bool {
    matches!(
        texture_type,
        TextureType::Normals
            | TextureType::NormalCamera
            | TextureType::Height
            | TextureType::Displacement
            | TextureType::Roughness
            | TextureType::Metalness
            | TextureType::AmbientOcclusion
            | TextureType::Shininess
            | TextureType::Specular
            | TextureType::Opacity
    )
}

fn bind(shader: &Shader, unit: u32, sampler: &str, flag: &str, texture: Option<&Texture>) {
    shader.set_uniform_1i(sampler, unit as i32);
    shader.set_uniform_1i(flag, texture.is_some() as i32);
    if let Some(texture) = texture {
        texture.bind_to(unit);
    }
}
