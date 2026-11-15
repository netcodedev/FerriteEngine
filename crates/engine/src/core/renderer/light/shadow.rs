//! Shadows of point and spot lights: a depth cube map per light (one array
//! texture for all of them), drawn from the light's position with six 90°
//! views like the faces of a die.
//!
//! Only the `MAX_SHADOWED_LIGHTS` shadow casting lights nearest the camera
//! get one each frame (`assign`). Drawing six views per light is the
//! expensive part, so at most `FACE_BUDGET` faces are drawn per frame
//! (`ShadowPlanner`): a light that is new to its layer or has moved gets
//! all its faces at once, the others are refreshed in turn, which keeps
//! static lights exact and moving things (the player walking past a
//! lantern) up to date every frame or two. A spot light only draws the
//! faces its cone reaches.
//!
//! The lit shaders look the maps up in `lightShadow` (`pbr/lighting.glsl`),
//! comparing against the depth a point at that distance would have had.

use cgmath::{perspective, Deg, InnerSpace, Matrix4, Point3, Vector3};

use super::light::{LightKind, LightSource};

/// Most lights with shadows per frame; must match `MAX_SHADOWED_LIGHTS`
/// in `pbr/lighting.glsl` only in that layers above it never appear.
pub const MAX_SHADOWED_LIGHTS: usize = 4;
/// Texels per side of each cube face.
pub const SHADOW_RESOLUTION: u32 = 512;
/// Near plane of the cube faces; must match `LIGHT_SHADOW_NEAR` in
/// `pbr/lighting.glsl`.
pub const SHADOW_NEAR: f32 = 0.05;
/// Faces drawn per frame at most, beyond those a light needs at once.
pub const FACE_BUDGET: usize = 12;
/// Texture unit the shadow cube maps are bound to while the scene is
/// shaded. Above the 16 every GL 4 driver has for sure, but the units below
/// are all taken; desktop GPUs have 32 or more.
pub const LIGHT_SHADOW_UNIT: u32 = 16;

/// Direction and up vector of each cube face, in the GL cube map order
/// (+X, −X, +Y, −Y, +Z, −Z).
const FACES: [([f32; 3], [f32; 3]); 6] = [
    ([1.0, 0.0, 0.0], [0.0, -1.0, 0.0]),
    ([-1.0, 0.0, 0.0], [0.0, -1.0, 0.0]),
    ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
    ([0.0, -1.0, 0.0], [0.0, 0.0, -1.0]),
    ([0.0, 0.0, 1.0], [0.0, -1.0, 0.0]),
    ([0.0, 0.0, -1.0], [0.0, -1.0, 0.0]),
];

/// World to clip space of `face` of a light at `position` reaching `range`.
pub fn face_view_projection(position: Point3<f32>, range: f32, face: usize) -> Matrix4<f32> {
    let (dir, up) = FACES[face];
    let view = Matrix4::look_at_rh(position, position + Vector3::from(dir), Vector3::from(up));
    let far = range.max(SHADOW_NEAR * 2.0);
    perspective(Deg(90.0), 1.0, SHADOW_NEAR, far) * view
}

/// The faces a light's shadow needs: all six for a point light; for a spot
/// light those its cone reaches (a face's view reaches 54.7° off its axis,
/// in the corners).
pub fn faces_needed(light: &LightSource) -> Vec<usize> {
    (0..6)
        .filter(|&face| match light.kind {
            LightKind::Spot => {
                let axis = Vector3::from(FACES[face].0);
                let off_axis = axis.dot(light.direction).clamp(-1.0, 1.0).acos();
                let cone = light.cos_outer.clamp(-1.0, 1.0).acos();
                off_axis <= cone + (1.0f32 / 3f32.sqrt()).acos() + 1e-4
            }
            _ => true,
        })
        .collect()
}

/// Give the nearest shadow casting lights a layer each (`shadow_layer`).
pub fn assign(lights: &mut [LightSource], eye: Point3<f32>) {
    let mut casters: Vec<usize> = (0..lights.len())
        .filter(|&i| lights[i].casts_shadows && lights[i].kind != LightKind::Directional)
        .collect();
    casters.sort_by(|&a, &b| lights[b].priority(eye).total_cmp(&lights[a].priority(eye)));
    for light in lights.iter_mut() {
        light.shadow_layer = None;
    }
    for (layer, &i) in casters.iter().take(MAX_SHADOWED_LIGHTS).enumerate() {
        lights[i].shadow_layer = Some(layer as u32);
    }
}

/// What a layer's shadow was drawn for: when it changes, the old map is
/// wrong and every face has to be drawn again.
#[derive(Clone, Copy, Debug, PartialEq)]
struct LayerKey {
    position: [f32; 3],
    direction: [f32; 3],
    range: f32,
    cos_outer: f32,
    kind: LightKind,
}

impl LayerKey {
    fn of(light: &LightSource) -> Self {
        Self {
            position: light.position.into(),
            direction: light.direction.into(),
            range: light.range,
            cos_outer: light.cos_outer,
            kind: light.kind,
        }
    }
}

/// Decides which cube faces are drawn this frame.
#[derive(Default)]
pub struct ShadowPlanner {
    keys: [Option<LayerKey>; MAX_SHADOWED_LIGHTS],
    /// Where the round of refreshes goes on next frame.
    cursor: usize,
}

impl ShadowPlanner {
    /// The `(layer, face)` pairs to draw for `lights` (with layers
    /// assigned) this frame.
    pub fn plan(&mut self, lights: &[LightSource]) -> Vec<(u32, usize)> {
        let mut layers: Vec<(u32, &LightSource)> = lights
            .iter()
            .filter_map(|l| l.shadow_layer.map(|layer| (layer, l)))
            .collect();
        layers.sort_by_key(|(layer, _)| *layer);

        let mut fresh = Vec::new();
        let mut refresh = Vec::new();
        let mut seen = [false; MAX_SHADOWED_LIGHTS];
        for (layer, light) in layers {
            let slot = layer as usize;
            seen[slot] = true;
            let key = LayerKey::of(light);
            let faces = faces_needed(light);
            if self.keys[slot] != Some(key) {
                self.keys[slot] = Some(key);
                fresh.extend(faces.into_iter().map(|face| (layer, face)));
            } else {
                refresh.extend(faces.into_iter().map(|face| (layer, face)));
            }
        }
        for (slot, seen) in seen.iter().enumerate() {
            if !seen {
                self.keys[slot] = None;
            }
        }

        let budget = FACE_BUDGET.saturating_sub(fresh.len()).min(refresh.len());
        if !refresh.is_empty() {
            let start = self.cursor % refresh.len();
            fresh.extend((0..budget).map(|i| refresh[(start + i) % refresh.len()]));
            self.cursor = (start + budget) % refresh.len();
        }
        fresh
    }
}

/// The depth cube maps of the shadowed lights (a cube map array, one cube
/// per layer) and the framebuffer they are drawn with.
pub struct LightShadowMaps {
    texture: gl::types::GLuint,
    framebuffer: gl::types::GLuint,
}

impl LightShadowMaps {
    pub fn new() -> Self {
        let (mut texture, mut framebuffer) = (0, 0);
        let size = SHADOW_RESOLUTION as i32;
        unsafe {
            gl::GenTextures(1, &mut texture);
            gl::BindTexture(gl::TEXTURE_CUBE_MAP_ARRAY, texture);
            gl::TexImage3D(
                gl::TEXTURE_CUBE_MAP_ARRAY,
                0,
                gl::DEPTH_COMPONENT24 as i32,
                size,
                size,
                (MAX_SHADOWED_LIGHTS * 6) as i32,
                0,
                gl::DEPTH_COMPONENT,
                gl::FLOAT,
                std::ptr::null(),
            );
            let linear = gl::LINEAR as i32;
            gl::TexParameteri(gl::TEXTURE_CUBE_MAP_ARRAY, gl::TEXTURE_MIN_FILTER, linear);
            gl::TexParameteri(gl::TEXTURE_CUBE_MAP_ARRAY, gl::TEXTURE_MAG_FILTER, linear);
            let clamp = gl::CLAMP_TO_EDGE as i32;
            gl::TexParameteri(gl::TEXTURE_CUBE_MAP_ARRAY, gl::TEXTURE_WRAP_S, clamp);
            gl::TexParameteri(gl::TEXTURE_CUBE_MAP_ARRAY, gl::TEXTURE_WRAP_T, clamp);
            gl::TexParameteri(gl::TEXTURE_CUBE_MAP_ARRAY, gl::TEXTURE_WRAP_R, clamp);
            // Hardware comparison: the shader gets how much of the 2×2
            // texels around the point is lit.
            gl::TexParameteri(
                gl::TEXTURE_CUBE_MAP_ARRAY,
                gl::TEXTURE_COMPARE_MODE,
                gl::COMPARE_REF_TO_TEXTURE as i32,
            );
            gl::TexParameteri(
                gl::TEXTURE_CUBE_MAP_ARRAY,
                gl::TEXTURE_COMPARE_FUNC,
                gl::LEQUAL as i32,
            );
            gl::BindTexture(gl::TEXTURE_CUBE_MAP_ARRAY, 0);
            // Filtering across the edges of the faces, not within each.
            gl::Enable(gl::TEXTURE_CUBE_MAP_SEAMLESS);

            gl::GenFramebuffers(1, &mut framebuffer);
            gl::BindFramebuffer(gl::FRAMEBUFFER, framebuffer);
            gl::DrawBuffer(gl::NONE);
            gl::ReadBuffer(gl::NONE);
            gl::BindFramebuffer(gl::FRAMEBUFFER, 0);
        }
        Self {
            texture,
            framebuffer,
        }
    }

    /// Draw into `face` of `layer`'s cube from now on, cleared.
    pub fn begin_face(&self, layer: u32, face: usize) {
        unsafe {
            gl::BindFramebuffer(gl::FRAMEBUFFER, self.framebuffer);
            gl::FramebufferTextureLayer(
                gl::FRAMEBUFFER,
                gl::DEPTH_ATTACHMENT,
                self.texture,
                0,
                (layer as usize * 6 + face) as i32,
            );
            let size = SHADOW_RESOLUTION as i32;
            gl::Viewport(0, 0, size, size);
            gl::DepthMask(gl::TRUE);
            gl::Enable(gl::DEPTH_TEST);
            gl::Clear(gl::DEPTH_BUFFER_BIT);
        }
    }

    /// Unbound from its unit, before drawing into it (no feedback loop).
    pub fn unbind(&self) {
        unsafe {
            gl::ActiveTexture(gl::TEXTURE0 + LIGHT_SHADOW_UNIT);
            gl::BindTexture(gl::TEXTURE_CUBE_MAP_ARRAY, 0);
            gl::ActiveTexture(gl::TEXTURE0);
        }
    }

    /// Bound to `LIGHT_SHADOW_UNIT` for the lit shaders.
    pub fn bind(&self) {
        unsafe {
            gl::ActiveTexture(gl::TEXTURE0 + LIGHT_SHADOW_UNIT);
            gl::BindTexture(gl::TEXTURE_CUBE_MAP_ARRAY, self.texture);
            gl::ActiveTexture(gl::TEXTURE0);
        }
    }
}

impl Drop for LightShadowMaps {
    fn drop(&mut self) {
        unsafe {
            gl::DeleteFramebuffers(1, &self.framebuffer);
            gl::DeleteTextures(1, &self.texture);
        }
    }
}

#[cfg(test)]
mod tests {
    use cgmath::{EuclideanSpace, Vector4};

    use super::*;
    use crate::core::renderer::light::light::{Light, LightSettings};

    fn point_at(x: f32) -> LightSource {
        Light::point([1.0; 3], 5.0, 10.0)
            .source(&Matrix4::from_translation(Vector3::new(x, 0.0, 0.0)))
            .unwrap()
    }

    fn spot(outer: f32) -> LightSource {
        Light::new(LightSettings {
            kind: LightKind::Spot,
            inner_angle: outer,
            outer_angle: outer,
            ..LightSettings::default()
        })
        .source(&Matrix4::from_scale(1.0))
        .unwrap()
    }

    #[test]
    fn each_face_looks_along_its_axis_out_to_the_range() {
        let position = Point3::new(1.0, 2.0, 3.0);
        for face in 0..6 {
            let vp = face_view_projection(position, 10.0, face);
            let axis = Vector3::from(FACES[face].0);
            let ahead = position + axis * 5.0;
            let clip = vp * ahead.to_homogeneous();
            let ndc = clip.truncate() / clip.w;
            assert!(ndc.x.abs() < 1e-4 && ndc.y.abs() < 1e-4, "face {face}: {ndc:?}");
            assert!(ndc.z > -1.0 && ndc.z < 1.0);
            let beyond = vp * (position + axis * 11.0).to_homogeneous();
            assert!(beyond.z / beyond.w > 1.0, "face {face} reaches past the range");
        }
    }

    #[test]
    fn the_six_faces_cover_every_direction() {
        let position = Point3::origin();
        for dir in [
            Vector3::new(0.3, 0.9, -0.2),
            Vector3::new(-1.0, -1.0, 1.0),
            Vector3::new(0.0, 0.0, -1.0),
        ] {
            let p = position + dir.normalize() * 3.0;
            let inside = (0..6).any(|face| {
                let c = face_view_projection(position, 10.0, face) * Vector4::new(p.x, p.y, p.z, 1.0);
                c.w > 0.0 && (c.x / c.w).abs() <= 1.0 && (c.y / c.w).abs() <= 1.0
            });
            assert!(inside, "{dir:?}");
        }
    }

    #[test]
    fn a_spot_light_draws_only_the_faces_its_cone_reaches() {
        // Straight down and narrow: inside the bottom face alone.
        assert_eq!(faces_needed(&spot(10.0)), vec![3]);
        // Wide: the sides too, never the top.
        assert_eq!(faces_needed(&spot(60.0)), vec![0, 1, 3, 4, 5]);
        assert_eq!(faces_needed(&point_at(0.0)).len(), 6);
    }

    #[test]
    fn the_nearest_casters_get_layers() {
        let mut lights: Vec<LightSource> = (0..6).map(|i| point_at(i as f32 * 30.0)).collect();
        lights[1].casts_shadows = false;
        assign(&mut lights, Point3::origin());
        let layers: Vec<Option<u32>> = lights.iter().map(|l| l.shadow_layer).collect();
        assert_eq!(layers, [Some(0), None, Some(1), Some(2), Some(3), None]);
    }

    #[test]
    fn directional_lights_never_get_shadow_maps() {
        let sun = Light::new(LightSettings {
            kind: LightKind::Directional,
            ..LightSettings::default()
        })
        .source(&Matrix4::from_scale(1.0))
        .unwrap();
        assert!(!sun.casts_shadows);
        let mut lights = vec![sun];
        assign(&mut lights, Point3::origin());
        assert_eq!(lights[0].shadow_layer, None);
    }

    #[test]
    fn new_lights_are_drawn_whole_then_refreshed_in_turn_within_the_budget() {
        let mut lights: Vec<LightSource> = (0..4).map(|i| point_at(i as f32)).collect();
        assign(&mut lights, Point3::origin());
        let mut planner = ShadowPlanner::default();

        // First frame: all 24 faces, the map would be wrong otherwise.
        assert_eq!(planner.plan(&lights).len(), 24);
        // Then the budget, going round so every face comes up.
        let mut drawn = std::collections::HashSet::new();
        for _ in 0..2 {
            let frame = planner.plan(&lights);
            assert_eq!(frame.len(), FACE_BUDGET);
            drawn.extend(frame);
        }
        assert_eq!(drawn.len(), 24);

        // A moved light is drawn whole at once, plus what the budget leaves.
        lights[2].position.y += 1.0;
        let frame = planner.plan(&lights);
        assert!((0..6).all(|face| frame.contains(&(2, face))));
        assert_eq!(frame.len(), FACE_BUDGET);
    }

    #[test]
    fn packed_lights_tell_the_shader_their_layer() {
        let mut light = point_at(0.0);
        assert_eq!(light.packed()[3][2], -1.0);
        light.shadow_layer = Some(3);
        assert_eq!(light.packed()[3][2], 3.0);
    }
}
