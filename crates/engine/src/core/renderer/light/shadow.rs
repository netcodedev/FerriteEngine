//! Shadows of point and spot lights: a depth cube map per light (one array
//! texture for all of them), drawn from the light's position with six 90°
//! views like the faces of a die.
//!
//! Only the `MAX_SHADOWED_LIGHTS` shadow casting lights nearest the camera
//! get one each frame (`assign`). Drawing six views per light is the
//! expensive part, so at most `FACE_BUDGET` faces are drawn per frame
//! (`ShadowPlanner`): a light that is new to its layer or has moved gets
//! all its faces at once, the others are refreshed in turn (a few faces
//! each frame even while moving lights take the whole budget), which keeps
//! static lights exact and moving things (the player walking past a
//! lantern) up to date every frame or two. A spot light only draws the
//! faces its cone reaches.
//!
//! The lit shaders look the maps up in `lightShadow` (`pbr/lighting.glsl`),
//! comparing against the depth a point at that distance would have had.

use cgmath::{perspective, Deg, InnerSpace, Matrix4, Point3, Vector3};

use super::light::{LightKind, LightSource};
use crate::core::gfx;

/// Most lights with shadows per frame; must match `MAX_SHADOWED_LIGHTS`
/// in `pbr/lighting.glsl` only in that layers above it never appear.
pub const MAX_SHADOWED_LIGHTS: usize = 8;
/// Texels per side of each cube face, unless the graphics settings say
/// otherwise.
pub const SHADOW_RESOLUTION: u32 = 512;
/// Near plane of the cube faces; must match `LIGHT_SHADOW_NEAR` in
/// `pbr/lighting.glsl`.
pub const SHADOW_NEAR: f32 = 0.05;
/// Faces drawn per frame at most, beyond those a light needs at once.
pub const FACE_BUDGET: usize = 12;
/// Faces of unchanged lights refreshed per frame even when moving lights
/// take the whole budget, so their maps never stay stale for long.
pub const MIN_REFRESH: usize = 2;
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
    // The graphics settings may give fewer lights a shadow.
    let most = crate::core::graphics::engine().point_shadow_lights.min(MAX_SHADOWED_LIGHTS);
    for (layer, &i) in casters.iter().take(most).enumerate() {
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

        let budget = FACE_BUDGET
            .saturating_sub(fresh.len())
            .max(MIN_REFRESH)
            .min(refresh.len());
        if !refresh.is_empty() {
            let start = self.cursor % refresh.len();
            fresh.extend((0..budget).map(|i| refresh[(start + i) % refresh.len()]));
            self.cursor = (start + budget) % refresh.len();
        }
        fresh
    }
}

/// The depth cube maps of the shadowed lights (a cube map array, one cube
/// per layer) and a render target per face of each.
pub struct LightShadowMaps {
    texture: gfx::Texture,
    /// By `layer * 6 + face`.
    faces: Vec<gfx::RenderTarget>,
    /// Texels per side of a face.
    size: u32,
}

/// Hardware comparison: the shader gets how much of the 2×2 texels around
/// the point is lit.
const SAMPLER: gfx::SamplerDesc = gfx::SamplerDesc::LINEAR_CLAMP.compare(gfx::CompareFunc::LessEqual);

impl LightShadowMaps {
    /// Faces of `size` texels per side.
    pub fn new(size: u32) -> Self {
        let desc = gfx::TextureDesc {
            kind: gfx::TextureKind::CubeArray,
            layers: (MAX_SHADOWED_LIGHTS * 6) as u32,
            ..gfx::TextureDesc::d2(gfx::Format::Depth24, size, size)
        };
        let texture = gfx::Texture::new(desc);
        let faces = (0..desc.layers)
            .map(|layer| gfx::RenderTarget::new(&[], Some(texture.attachment(0, Some(layer)))))
            .collect();
        Self { texture, faces, size }
    }

    pub fn size(&self) -> u32 {
        self.size
    }

    /// A pass drawing into `face` of `layer`'s cube, cleared; end it with
    /// `end_face`.
    pub fn begin_face(&self, layer: u32, face: usize) {
        if let Some(target) = self.faces.get(layer as usize * 6 + face) {
            gfx::begin_pass(
                target.reference(),
                (self.size, self.size),
                gfx::Load::Keep,
                gfx::Load::Depth(1.0),
            );
        }
    }

    /// End the pass `begin_face` began.
    pub fn end_face(&self) {
        gfx::end_pass();
    }

    /// Unbound from its unit, before drawing into it (no feedback loop).
    pub fn unbind(&self) {
        gfx::unbind_texture(LIGHT_SHADOW_UNIT, gfx::TextureKind::CubeArray);
    }

    /// Bound to `LIGHT_SHADOW_UNIT` for the lit shaders.
    pub fn bind(&self) {
        gfx::bind_texture(LIGHT_SHADOW_UNIT, &self.texture, &SAMPLER);
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
        let count = MAX_SHADOWED_LIGHTS + 2;
        let mut lights: Vec<LightSource> = (0..count).map(|i| point_at(i as f32 * 30.0)).collect();
        lights[1].casts_shadows = false;
        assign(&mut lights, Point3::origin());
        let layers: Vec<Option<u32>> = lights.iter().map(|l| l.shadow_layer).collect();
        // The one that casts none is skipped, the farthest gets none.
        let mut want: Vec<Option<u32>> = vec![Some(0), None];
        want.extend((1..MAX_SHADOWED_LIGHTS as u32).map(Some));
        want.push(None);
        assert_eq!(layers, want);
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
    fn moving_lights_never_starve_the_refresh_of_static_ones() {
        let mut lights: Vec<LightSource> = (0..4).map(|i| point_at(i as f32)).collect();
        assign(&mut lights, Point3::origin());
        let mut planner = ShadowPlanner::default();
        planner.plan(&lights);

        // Layers 0 and 1 move every frame (12 fresh faces); layers 2 and 3
        // still come round in turn.
        let mut refreshed = std::collections::HashSet::new();
        for frame in 0..6 {
            for light in &mut lights[..2] {
                light.position.y += 0.1;
            }
            let faces = planner.plan(&lights);
            assert_eq!(faces.len(), 12 + MIN_REFRESH, "frame {frame}");
            refreshed.extend(faces.into_iter().filter(|(layer, _)| *layer >= 2));
        }
        assert_eq!(refreshed.len(), 12);
    }

    #[test]
    fn packed_lights_tell_the_shader_their_layer() {
        let mut light = point_at(0.0);
        assert_eq!(light.packed()[3][2], -1.0);
        light.shadow_layer = Some(3);
        assert_eq!(light.packed()[3][2], 3.0);
    }
}
