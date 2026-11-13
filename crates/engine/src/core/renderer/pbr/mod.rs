//! Physically based shading of model materials: the light a scene gives a
//! PBR shader, and binding a material's maps.

use std::collections::HashMap;

use cgmath::{EuclideanSpace, InnerSpace, Matrix4, Point3, Vector3};
use russimp_ng::material::TextureType;

use crate::core::{
    renderer::{light::skylight::SkyLight, shader::Shader, texture::Texture},
    scene::Scene,
};

const PBR_GLSL: &str = include_str!("pbr.glsl");

/// Texture unit the scene binds the sun's shadow map to.
const SHADOW_MAP_UNIT: i32 = 15;

/// A fragment shader with the shared PBR code (`shadePbr`) put in front of
/// `body`.
pub fn fragment_source(body: &str) -> String {
    format!("#version 330 core\n{PBR_GLSL}\n{body}")
}

/// The light a PBR shader is drawn with.
pub struct Lighting {
    /// Toward the sun.
    pub sun_direction: Vector3<f32>,
    pub camera_position: Point3<f32>,
    pub cloud_coverage: f32,
    /// The sun's view-projection, when the scene has a shadow map.
    pub light_projection: Option<Matrix4<f32>>,
}

impl Lighting {
    /// The scene's sun, sky and camera. `None` without a sky light.
    pub fn from_scene(scene: &Scene) -> Option<Self> {
        let skylight = scene.get_component::<SkyLight>()?;
        let camera_position = scene
            .get_active_camera()
            .map(|camera| camera.get_camera().get_eye_position())
            .unwrap_or_else(Point3::origin);
        Some(Self {
            sun_direction: skylight.get_sun_direction().normalize(),
            camera_position,
            cloud_coverage: skylight.get_cloud_coverage(),
            light_projection: scene
                .get_shadow_map_size()
                .map(|_| skylight.get_projection()),
        })
    }

    /// Clear daylight from `sun_direction` without shadows, for previews.
    pub fn preview(sun_direction: Vector3<f32>, camera_position: Point3<f32>) -> Self {
        Self {
            sun_direction: sun_direction.normalize(),
            camera_position,
            cloud_coverage: 0.0,
            light_projection: None,
        }
    }

    /// Set the lighting uniforms of the bound `shader`.
    pub fn apply(&self, shader: &Shader) {
        // The scene draws the shadow pass into the shadow map's framebuffer
        // and everything else into the default one.
        let mut framebuffer = 0;
        unsafe { gl::GetIntegerv(gl::FRAMEBUFFER_BINDING, &mut framebuffer) };
        shader.set_uniform_1i("isShadowPass", (framebuffer != 0) as i32);
        shader.set_uniform_3fv("sunDirection", &self.sun_direction);
        let eye = self.camera_position;
        shader.set_uniform_3f("cameraPosition", eye.x, eye.y, eye.z);
        shader.set_uniform_1f("cloudCoverage", self.cloud_coverage);
        shader.set_uniform_1i("shadowMap", SHADOW_MAP_UNIT);
        match &self.light_projection {
            Some(projection) => {
                shader.set_uniform_mat4("lightProjection", projection);
                shader.set_uniform_1i("useShadowMap", 1);
            }
            None => shader.set_uniform_1i("useShadowMap", 0),
        }
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
        unsafe { gl::ActiveTexture(gl::TEXTURE0 + unit) };
        texture.bind();
    }
}
