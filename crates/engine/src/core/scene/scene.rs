use std::cell::{Cell, RefCell};

use cgmath::{InnerSpace, Matrix4, Point3, Quaternion, SquareMatrix, Vector3};
use glfw::{Action, Glfw, Key, WindowEvent};

use crate::core::{
    gfx,
    entity::{
        component::{
            camera_component::CameraComponent, missing_component::MissingComponent, Component,
        },
        Entity, EntityHandle,
    },
    physics::{collider_debug, physics_engine::PhysicsEngine, rigidbody::RigidBody},
    renderer::{
        framebuffer::{CascadedShadowMap, ShadowFrameBuffer},
        line::{Line, LineRenderer},
        plane::PlaneRenderer,
        text::TextRenderer,
        light::{
            clusters::{ClusterBuffers, ClusterCamera},
            light::{self as lights, LightBuffer},
            shadow::{self, LightShadowMaps, ShadowPlanner},
            skylight::{SkyLight, SHADOW_CASCADES},
        },
        debug_views,
        depth_pass::DepthPass,
        hdr::{self, Hdr, HdrSettings},
        profiler::{self, Pass},
        raytracing::{self, RayTracedShadows, ShadowView},
        sky::SkyRenderer,
        ssao::{Ssao, SSAO_UNIT},
        texture::TextureRenderer,
        weather::{self, WeatherMaps, WeatherTargets},
        pbr::Lighting,
    },
    scene::scene_file::ComponentData,
    script,
    view_frustum::ViewFrustum,
    window::Window,
};

use super::Scene;

/// Texture unit the sun's shadow cascades are bound to while the scene is
/// shaded (`shadowMap` in the lit shaders, see `pbr`).
const SHADOW_MAP_UNIT: u32 = 15;

/// Metres the camera may move in a frame before every cascade is drawn
/// again at once.
const CASCADE_JUMP: f32 = 5.0;

impl Scene {
    pub fn new() -> Self {
        Scene {
            entities: Vec::new(),
            physics_engine: PhysicsEngine::new(),
            shadow_fbo: None,
            terrain_debug_fbo: None,
            water_debug_fbo: None,
            texture_renderer: TextureRenderer::new(),
            show_shadow_debug: false,
            shadow_map_size: None,
            debug_map_size: None,
            editor_camera: None,
            physics_enabled: true,
            viewport: None,
            ssao: RefCell::new(None),
            hdr: RefCell::new(None),
            ambient_occlusion_origin: Cell::new(None),
            ray_traced: RefCell::new(None),
            ray_traced_shadows_origin: Cell::new(None),
            ray_traced_light_shadows_origin: Cell::new(None),
            weather: RefCell::new(None),
            weather_maps: Cell::new(None),
            lights: RefCell::new(None),
            light_clusters: RefCell::new(None),
            light_count: Cell::new(0),
            frame_lighting: Cell::new(None),
            drawn_cascades: Cell::new(None),
            shadow_frame: Cell::new(0),
            shadow_redraw: Cell::new(false),
            shadow_eye: Cell::new(None),
            light_sources: RefCell::new(Vec::new()),
            light_shadows: RefCell::new(None),
            shadow_planner: RefCell::new(ShadowPlanner::default()),
            deferred: Vec::new(),
            generation: {
                static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            },
        }
    }

    /// Run `update` on every root entity in order, each taken out of the
    /// scene while it runs. The update may remove, add or move root entities,
    /// or replace the whole scene: the entity goes back where it was taken
    /// from, wherever that is now, so none is skipped and the loop never
    /// steps past the end. After the scene was replaced, the entity belonged
    /// to the old one and is dropped with it.
    ///
    /// While it runs, an empty placeholder holds its place (transient and
    /// hidden: neither saved, listed nor drawn, and without components or
    /// children), so taking it out and putting it back costs a swap instead
    /// of moving every later root twice.
    fn update_roots(&mut self, mut update: impl FnMut(&mut Entity, &mut Scene)) {
        fn placeholder() -> Entity {
            let mut placeholder = Entity::new("");
            placeholder.set_transient(true);
            placeholder.set_hidden(true);
            placeholder
        }
        let generation = self.generation;
        let mut spare = placeholder();
        let mut i = 0;
        while i < self.entities.len() {
            let held = spare.id;
            let mut entity = std::mem::replace(&mut self.entities[i], spare);
            update(&mut entity, self);
            if self.generation != generation {
                return;
            }
            let found = if self.entities.get(i).is_some_and(|e| e.id == held) {
                Some(i)
            } else {
                self.entities.iter().position(|e| e.id == held)
            };
            i = match found {
                Some(at) => {
                    spare = std::mem::replace(&mut self.entities[at], entity);
                    at + 1
                }
                // The update took the placeholder out itself (with a sweep
                // over all roots, say): the entity goes where it was.
                None => {
                    let at = i.min(self.entities.len());
                    self.entities.insert(at, entity);
                    spare = placeholder();
                    at + 1
                }
            };
        }
    }

    /// Do `work` once every entity is updated this frame. While an entity
    /// (and what is below it) is updated, it is out of the scene: whatever
    /// needs to reach other entities through the scene, from any entity,
    /// goes here.
    pub fn defer(&mut self, work: impl FnOnce(&mut Scene) + 'static) {
        self.deferred.push(Box::new(work));
    }

    /// The queued work, and the work that queues, until there is none.
    fn run_deferred(&mut self) {
        while !self.deferred.is_empty() {
            for work in std::mem::take(&mut self.deferred) {
                work(self);
            }
        }
    }

    /// Build every script component again with the scripts installed now
    /// (after a game library was reloaded): from its data, which it hands
    /// over itself, or from what a placeholder kept for a script that was
    /// not loaded. Returns one message per script that could not be built.
    pub fn rebuild_scripts(&mut self) -> Vec<String> {
        fn visit(entity: &mut Entity, physics: &mut PhysicsEngine, problems: &mut Vec<String>) {
            for index in 0..entity.components().len() {
                let component = &entity.components()[index];
                let data = match component.as_any().downcast_ref::<MissingComponent>() {
                    Some(missing) => Some(missing.data().clone()),
                    None => component.to_data(),
                };
                let Some(data @ ComponentData::Script { .. }) = data else {
                    continue;
                };
                let ComponentData::Script { name, data: value } = &data else {
                    continue;
                };
                let rebuilt: Box<dyn Component> = match script::create(name, value) {
                    Ok(component) => component,
                    Err(e) => {
                        problems.push(format!("{}: {e}", entity.get_name()));
                        Box::new(MissingComponent::new(data.clone(), e))
                    }
                };
                if let Some(mut old) = entity.replace_component_at(index, rebuilt) {
                    old.on_remove(physics);
                }
            }
            for child in entity.get_children_mut() {
                visit(child, physics, problems);
            }
        }
        let mut problems = Vec::new();
        for entity in self.entities.iter_mut() {
            visit(entity, &mut self.physics_engine, &mut problems);
        }
        problems
    }

    /// The entities and the physics world at once, for code that changes
    /// both (e.g. removes a collider an entity's component owns).
    pub fn entities_and_physics_mut(&mut self) -> (&mut Vec<Entity>, &mut PhysicsEngine) {
        (&mut self.entities, &mut self.physics_engine)
    }

    /// Become `other` (a scene a server sent, say), keeping how this one is
    /// shown and run: its viewport, editor camera and whether physics runs.
    pub fn replace_with(&mut self, mut other: Scene) {
        other.viewport = self.viewport;
        other.editor_camera = self.editor_camera.take();
        other.physics_enabled = self.physics_enabled;
        *self = other;
    }

    pub fn add_shadow_map(&mut self, width: u32, height: u32) {
        self.shadow_map_size = Some((width, height));
        if crate::core::headless::is_headless() {
            return;
        }
        // One square layer per cascade. The scene only says that it has
        // shadows; how big they are is a graphics setting.
        let size = crate::core::graphics::engine().shadow_map_size;
        self.shadow_fbo = Some(CascadedShadowMap::new(size, SHADOW_CASCADES as u32));
        self.drawn_cascades.set(None);
        self.shadow_map_size = Some((width, height));
    }

    /// Create the camera-perspective debug FBOs used by F10.
    /// `width`/`height` should match the window dimensions.
    pub fn add_debug_maps(&mut self, width: u32, height: u32) {
        self.debug_map_size = Some((width, height));
        if crate::core::headless::is_headless() {
            return;
        }
        self.terrain_debug_fbo = Some(ShadowFrameBuffer::new(width, height));
        self.water_debug_fbo   = Some(ShadowFrameBuffer::new(width, height));
        self.debug_map_size = Some((width, height));
    }

    pub fn get_shadow_map_size(&self) -> Option<(u32, u32)> {
        self.shadow_map_size
    }

    pub fn get_debug_map_size(&self) -> Option<(u32, u32)> {
        self.debug_map_size
    }

    /// The camera everything renders from: the editor camera when one is
    /// set, otherwise the first `CameraComponent` in the scene.
    /// Have every cascade of the sun's shadows drawn again this frame
    /// (else the far ones are drawn every few frames): asked for by what
    /// changed shape since, from its draw into the nearest cascade (drawn
    /// first, every frame), so the far ones do not show its old shadow on
    /// its new shape. The terrain asks when a chunk changes its level of
    /// detail, or is new.
    pub fn request_shadow_redraw(&self) {
        self.shadow_redraw.set(true);
    }

    pub fn get_active_camera(&self) -> Option<&CameraComponent> {
        self.editor_camera
            .as_ref()
            .or_else(|| self.get_component::<CameraComponent>())
    }

    pub fn set_editor_camera(&mut self, camera: Option<CameraComponent>) {
        self.editor_camera = camera;
    }

    pub fn get_editor_camera_mut(&mut self) -> Option<&mut CameraComponent> {
        self.editor_camera.as_mut()
    }

    pub fn set_physics_enabled(&mut self, enabled: bool) {
        self.physics_enabled = enabled;
    }

    pub fn set_viewport(&mut self, viewport: Option<[i32; 4]>) {
        self.viewport = viewport;
    }

    /// Window position of the ambient occlusion while the camera's view is
    /// shaded with it (it is bound to `ssao::SSAO_UNIT` then).
    pub fn ambient_occlusion_origin(&self) -> Option<[i32; 2]> {
        self.ambient_occlusion_origin.get()
    }

    /// Window position of the ray traced sun shadows while the camera's
    /// view is shaded with them (bound to `raytracing::RT_SHADOW_UNIT`).
    pub fn ray_traced_shadows_origin(&self) -> Option<[i32; 2]> {
        self.ray_traced_shadows_origin.get()
    }

    /// Window position of the ray traced lamp shadows while the camera's
    /// view is shaded with them (bound to `raytracing::RT_LIGHT_SHADOW_UNIT`).
    pub fn ray_traced_light_shadows_origin(&self) -> Option<[i32; 2]> {
        self.ray_traced_light_shadows_origin.get()
    }

    /// Whether the device can trace rays for this scene; makes what
    /// tracing needs on first use.
    fn can_ray_trace(&self) -> bool {
        let mut ray_traced = self.ray_traced.borrow_mut();
        if ray_traced.is_none() {
            *ray_traced = RayTracedShadows::new();
        }
        ray_traced.is_some()
    }

    /// Whether this frame's sun shadows are ray traced: asked for in the
    /// settings, the device can, and there is a sun and a camera.
    fn wants_ray_traced_shadows(&self) -> bool {
        crate::core::graphics::engine().ray_traced_shadows
            && self.shadow_fbo.is_some()
            && self.get_component::<SkyLight>().is_some()
            && self.get_active_camera().is_some()
            && self.can_ray_trace()
    }

    /// Whether this frame's ambient occlusion is ray traced (instead of
    /// SSAO): asked for in the settings, the sky light has it on, the
    /// device can, and there is a camera.
    fn wants_ray_traced_ao(&self) -> bool {
        crate::core::graphics::engine().ambient_occlusion == crate::core::graphics::AoMode::RayTraced
            && self
                .get_component::<SkyLight>()
                .is_some_and(|skylight| skylight.ambient_occlusion().enabled)
            && self.get_active_camera().is_some()
            && self.can_ray_trace()
    }

    /// Whether this frame's lamp shadows are ray traced (instead of their
    /// cube maps): asked for in the settings, the device can, and there is
    /// a camera.
    fn wants_ray_traced_lamps(&self) -> bool {
        crate::core::graphics::engine().ray_traced_lights
            && self.get_active_camera().is_some()
            && self.can_ray_trace()
    }

    /// Hand the scene's meshes around the camera to the ray traced
    /// shadows and build what the rays are traced through.
    fn collect_ray_traced_scene(&self, window: &Window) {
        let Some(camera) = self.get_active_camera() else {
            return;
        };
        let mut ray_traced = self.ray_traced.borrow_mut();
        let Some(ray_traced) = ray_traced.as_mut() else {
            return;
        };
        let eye = camera.get_camera().get_eye_position();
        // As far as terrain is loaded, and a little beyond for what stands
        // at its edge.
        let radius = crate::core::graphics::engine().load_distance + 128.0;
        let view_projection = raytracing::collect_view(eye, radius);
        ray_traced.begin_collect_pass();
        raytracing::begin_collect();
        {
            let _depth_only = DepthPass::begin();
            for entity in self.entities.iter() {
                entity.render(self, &view_projection, Matrix4::identity());
            }
        }
        let instances = raytracing::end_collect();
        gfx::end_pass();
        ray_traced.build(&instances);
        self.apply_viewport(window);
    }

    /// Where the weather's maps lie while the scene is shaded with them
    /// (bound to `weather::EXPOSURE_UNIT` and `weather::SNOW_TRAIL_UNIT`).
    pub fn weather_maps(&self) -> Option<WeatherMaps> {
        self.weather_maps.get()
    }

    /// How many of the scene's lights the lit shaders get while it is drawn
    /// (see `light::light::select`), 0 otherwise.
    pub fn light_count(&self) -> usize {
        self.light_count.get()
    }

    /// The lighting looked up at the start of the frame being drawn, if one
    /// is (see `frame_lighting`).
    pub fn frame_lighting(&self) -> Option<Lighting> {
        self.frame_lighting.get()
    }

    /// Gather the scene's lights, keep the ones that matter for the camera
    /// and upload them for the shaders.
    fn upload_lights(&self) {
        self.light_count.set(0);
        self.light_sources.borrow_mut().clear();
        let Some(camera) = self.get_active_camera() else {
            return;
        };
        let eye = camera.get_camera().get_eye_position();
        let found = lights::gather(self, eye);
        if found.is_empty() && self.lights.borrow().is_none() {
            return;
        }
        let mut selected = lights::select(found, eye);
        // With ray traced lamp shadows no lamp gets a cube map.
        if !self.wants_ray_traced_lamps() {
            shadow::assign(&mut selected, eye);
        }
        let mut buffer = self.lights.borrow_mut();
        buffer.get_or_insert_with(LightBuffer::new).upload(&selected);
        let projection = camera.get_projection();
        let cluster_camera = ClusterCamera {
            view: camera.get_camera().get_matrix(),
            projection: projection.get_matrix(),
            near: projection.znear,
            far: projection.get_zfar(),
        };
        self.light_clusters
            .borrow_mut()
            .get_or_insert_with(ClusterBuffers::new)
            .upload(&cluster_camera, &selected);
        self.light_count.set(selected.len());
        *self.light_sources.borrow_mut() = selected;
    }

    /// The shadow cube maps of the point and spot lights that get one this
    /// frame (see `light::shadow`): the faces due are drawn, depth only.
    fn render_light_shadows(&self, window: &Window) {
        let sources = self.light_sources.borrow();
        if sources.iter().all(|l| l.shadow_layer.is_none()) {
            return;
        }
        let mut maps = self.light_shadows.borrow_mut();
        // Their size is a graphics setting: new maps, every face drawn anew.
        let size = crate::core::graphics::engine().point_shadow_resolution;
        if maps.as_ref().is_some_and(|maps| maps.size() != size) {
            *maps = None;
            *self.shadow_planner.borrow_mut() = ShadowPlanner::default();
        }
        let maps = maps.get_or_insert_with(|| LightShadowMaps::new(size));
        maps.unbind();
        let plan = self.shadow_planner.borrow_mut().plan(&sources);
        let _depth_only = DepthPass::begin();
        for (layer, face) in plan {
            let Some(light) = sources.iter().find(|l| l.shadow_layer == Some(layer)) else {
                continue;
            };
            let view_projection = shadow::face_view_projection(light.position, light.range, face);
            maps.begin_face(layer, face);
            for entity in self.entities.iter() {
                entity.render(self, &view_projection, Matrix4::identity());
            }
            self.render_snow_layer(&view_projection, true);
            maps.end_face();
        }
        self.apply_viewport(window);
    }

    /// The view culling is frozen at (see `ViewFrustum::set_frozen`), as
    /// lines: its near rectangle and its edges out to a kilometre and a
    /// half.
    fn render_frozen_view(&self, view_projection: &Matrix4<f32>) {
        let Some(c) = ViewFrustum::frozen_corners(1500.0) else {
            return;
        };
        let mut lines = Vec::new();
        let mut line = |a: Point3<f32>, b: Point3<f32>| {
            let along = b - a;
            let length = along.magnitude();
            if length > 1e-4 {
                lines.push(Line::new(a, along / length, length));
            }
        };
        for i in 0..4 {
            let j = (i + 1) % 4;
            line(c[i], c[j]);
            line(c[4 + i], c[4 + j]);
            line(c[i], c[4 + i]);
        }
        LineRenderer::render_lines(view_projection, &lines, Vector3::new(4.0, 3.0, 0.4), false);
    }

    /// The debug views the editor switched on (see `debug_views`), over
    /// the tone mapped view.
    fn render_debug_views(&self, depth_prepass: bool) {
        if debug_views::ambient_occlusion() && depth_prepass {
            if let Some(texture) = self.ambient_occlusion_texture() {
                debug_views::draw_ambient_occlusion(texture as gfx::RawId);
            }
        }
        // Only while they are traced (the last ones stay otherwise).
        if debug_views::lamp_shadows() && crate::core::graphics::engine().ray_traced_lights {
            let lamps = self.ray_traced.borrow().as_ref().and_then(RayTracedShadows::lamp_result_texture);
            if let Some(texture) = lamps {
                debug_views::draw_colour(texture);
            }
        }
        if debug_views::sun_shadows() {
            if let Some(shadow_fbo) = &self.shadow_fbo {
                debug_views::draw_shadow_cascades(shadow_fbo.texture(), SHADOW_CASCADES);
            }
        }
    }

    /// GL name of the last frame's ambient occlusion (one red channel, as big
    /// as the viewport), for debug views.
    pub fn ambient_occlusion_texture(&self) -> Option<u32> {
        let ray_traced = (crate::core::graphics::engine().ambient_occlusion == crate::core::graphics::AoMode::RayTraced)
            .then(|| self.ray_traced.borrow().as_ref().and_then(RayTracedShadows::ao_result_texture))
            .flatten();
        ray_traced
            .map(|raw| raw as u32)
            .or_else(|| self.ssao.borrow().as_ref().and_then(Ssao::result_texture))
    }

    fn apply_viewport(&self, window: &Window) {
        match self.viewport {
            Some([x, y, w, h]) => gfx::set_viewport(gfx::Rect::new(x, y, w.max(1) as u32, h.max(1) as u32)),
            None => window.reset_viewport(),
        }
    }

    /// Edit-mode update: no physics, no gameplay — only components that keep
    /// the scene presentable (see `Component::update_in_editor`).
    pub fn update_in_editor(&mut self, delta_time: f64) {
        self.apply_graphics();
        self.physics_engine.paused_frame();
        self.update_roots(|entity, scene| entity.update_in_editor(scene, delta_time));
        self.run_deferred();
    }

    /// Returns whether the currently-bound framebuffer allows water rendering.
    /// Water is allowed on the default framebuffer (FBO 0) and on the
    /// dedicated water-debug FBO only.  All other FBOs (shadow map,
    /// terrain-debug) should skip the transparent water pass.
    pub fn is_water_render_allowed(&self) -> bool {
        hdr::draws_to_view() || self.is_water_depth_capture()
    }

    /// Returns true when rendering into the water depth-capture FBO.
    /// In this mode `render_transparent` writes depth (DepthMask=TRUE,
    /// GL_ALWAYS) instead of blending colour, so the depth texture shows
    /// exactly where the water surface sits in clip-space.
    pub fn is_water_depth_capture(&self) -> bool {
        let current = gfx::current_target();
        self.water_debug_fbo
            .as_ref()
            .is_some_and(|fbo| current.is_some() && fbo.0.reference() == current)
    }

    /// Follow the graphics settings where the scene's own targets depend
    /// on them (the sun's shadow map size). Run by `update` and
    /// `update_in_editor`.
    fn apply_graphics(&mut self) {
        let shadow_size = crate::core::graphics::engine().shadow_map_size;
        if self.shadow_fbo.as_ref().is_some_and(|fbo| fbo.layer_size() != shadow_size) {
            self.shadow_fbo = Some(CascadedShadowMap::new(shadow_size, SHADOW_CASCADES as u32));
            self.drawn_cascades.set(None);
        }
    }

    pub fn update(&mut self, delta_time: f64) {
        self.apply_graphics();
        let _probe = Probe(std::time::Instant::now(), &crate::core::spike_probe::UPDATE_US);
        self.physics_engine.begin_frame();
        // Messages in and out, and the other players where they are, before
        // anything moves.
        crate::core::net::update(self, delta_time);
        if self.physics_enabled {
            self.physics_engine.update((delta_time as f32).min(1.0 / 20.0));
        } else {
            self.physics_engine.paused_frame();
        }
        self.update_roots(|entity, scene| entity.update(scene, delta_time));
        self.run_deferred();
    }

    pub fn render(&self, window: &Window) {
        let _probe = Probe(std::time::Instant::now(), &crate::core::spike_probe::RENDER_US);
        // 2D overlays drawn by components (menus, HUD) lay out in the
        // viewport's pixels, as the cursor they get does (see
        // `handle_event`), not stretched from the whole window into it.
        let overlay_size = (PlaneRenderer::get_size(), TextRenderer::get_size());
        if let Some([_, _, w, h]) = self.viewport {
            PlaneRenderer::resize(w.max(1) as u32, h.max(1) as u32);
            TextRenderer::resize(w.max(1) as u32, h.max(1) as u32);
        }
        self.render_passes(window);
        let ((plane_w, plane_h), (text_w, text_h)) = overlay_size;
        PlaneRenderer::resize(plane_w as u32, plane_h as u32);
        TextRenderer::resize(text_w, text_h);
    }

    fn render_passes(&self, window: &Window) {
        // The clouds' noise, for the sky and the cloud shadows.
        crate::core::renderer::sky::noise::bind();
        self.frame_lighting.set(None);
        self.frame_lighting.set(Lighting::from_scene(self));
        // Every pass of the frame sways models by the same clock.
        crate::core::material::wind::set_clock(crate::core::renderer::weather::seconds());
        profiler::begin_frame();
        self.render_passes_lit(window);
        profiler::end_frame();
        // Whatever draws after the scene (previews, ...) has lights of its
        // own, if any: not the scene's grid.
        if let Some(clusters) = self.light_clusters.borrow().as_ref() {
            clusters.disable();
        }
        self.frame_lighting.set(None);
    }

    fn render_passes_lit(&self, window: &Window) {
        let parent_transform = Matrix4::identity();

        // The weather's maps first: the snow layer needs them in the shadow
        // pass too.
        profiler::mark(Pass::WeatherMaps);
        self.render_weather_maps(window);
        self.upload_lights();

        // Shadow Pass
        profiler::mark(Pass::SunShadows);
        let ray_traced = self.wants_ray_traced_shadows();
        let ray_traced_lamps = self.wants_ray_traced_lamps() && self.light_count.get() > 0;
        let ray_traced_ao = self.wants_ray_traced_ao();
        if ray_traced || ray_traced_lamps || ray_traced_ao {
            self.collect_ray_traced_scene(window);
        }
        if ray_traced {
            // The cascades are drawn anew once ray tracing is switched off.
            self.drawn_cascades.set(None);
        }
        if let (Some(shadow_fbo), false) = (&self.shadow_fbo, ray_traced) {
            if let Some(skylight) = self.get_component::<SkyLight>() {
                // Unbound while drawn into, to prevent a feedback loop.
                gfx::unbind_texture(SHADOW_MAP_UNIT, gfx::TextureKind::D2Array);
                let frame = self.shadow_frame.get();
                self.shadow_frame.set(frame.wrapping_add(1));
                let eye = self
                    .get_active_camera()
                    .map(|camera| camera.get_camera().get_eye_position());
                // A camera that jumped needs every cascade where it is now.
                let jumped = match (eye, self.shadow_eye.get()) {
                    (Some(eye), Some(last)) => (eye - last).magnitude() > CASCADE_JUMP,
                    _ => true,
                };
                self.shadow_eye.set(eye);
                let current = *skylight.shadow_cascades();
                let mut drawn = match self.drawn_cascades.get() {
                    Some(drawn) if !jumped => drawn,
                    _ => {
                        // All of them, this frame.
                        self.drawn_cascades.set(None);
                        current
                    }
                };
                let redraw_all = self.drawn_cascades.get().is_none();
                let intervals = skylight.shadows().intervals();
                let in_use = crate::core::graphics::engine().shadow_cascades;
                for (index, cascade) in current.iter().enumerate() {
                    // Cascades the settings leave out are not drawn (their
                    // projection covers nothing, see `SkyLight`).
                    if index >= in_use {
                        drawn[index] = *cascade;
                        continue;
                    }
                    // The far cascades change little from frame to frame and
                    // are drawn every few frames, staggered; the shaders use
                    // the projection each was drawn with.
                    let interval = intervals[index];
                    // Asked for while the nearest cascade was drawn, by what
                    // changed shape there (see `request_shadow_redraw`).
                    let asked = self.shadow_redraw.get();
                    if !redraw_all && !asked && (frame + index as u64) % interval != 0 {
                        continue;
                    }
                    drawn[index] = *cascade;
                    let _depth_only = DepthPass::begin();
                    shadow_fbo.begin_cascade(index as u32);
                    for entity in self.entities.iter() {
                        entity.render(self, &cascade.view_projection, parent_transform);
                    }
                    self.render_snow_layer(&cascade.view_projection, true);
                    gfx::end_pass();
                }
                self.drawn_cascades.set(Some(drawn));
                self.shadow_redraw.set(false);
                if let Some(mut lighting) = self.frame_lighting.get() {
                    lighting.shadow_cascades = Some(drawn);
                    self.frame_lighting.set(Some(lighting));
                }
                self.apply_viewport(window);
            }
        }

        profiler::mark(Pass::LightShadows);
        self.render_light_shadows(window);

        // The passes since may have bound other textures where the weather's
        // maps go.
        self.bind_weather_maps();

        // Render Pass
        self.apply_viewport(window);
        self.ambient_occlusion_origin.set(None);
        self.ray_traced_shadows_origin.set(None);
        self.ray_traced_light_shadows_origin.set(None);
        if let Some(camera) = self.get_active_camera() {
            let view_projection = camera.get_view_projection();
            // The camera's passes cull against its view (or the one culling
            // is frozen at, see `ViewFrustum::set_frozen`).
            ViewFrustum::begin_view(view_projection);

            // Everything the camera sees goes into the HDR target, as big as
            // the viewport, and is tone mapped into the viewport at the end.
            let mut hdr_settings = self
                .get_component::<SkyLight>()
                .map_or_else(HdrSettings::default, |skylight| *skylight.hdr());
            // The graphics settings may switch bloom off, never on.
            let knobs = crate::core::graphics::engine();
            hdr_settings.bloom &= knobs.bloom;
            let mut hdr = self.hdr.borrow_mut();
            let hdr = hdr.get_or_insert_with(Hdr::new);
            profiler::mark(Pass::DepthPrepass);
            hdr.begin();
            // With TAA the view is drawn with a sub-pixel jitter.
            let view_projection = hdr.view_projection(view_projection);

            // Ambient occlusion — the opaque scene's depth from the camera,
            // and from it the occlusion the lit shaders darken their ambient
            // light by. The depth is drawn into the HDR target and stays
            // there: the shading pass below only shades the visible surface
            // of each pixel (depth test LEQUAL against it).
            let ambient_occlusion = self
                .get_component::<SkyLight>()
                .map(|skylight| *skylight.ambient_occlusion())
                .filter(|settings| {
                    settings.enabled
                        && match knobs.ambient_occlusion {
                            crate::core::graphics::AoMode::Off => false,
                            crate::core::graphics::AoMode::RayTraced if ray_traced_ao => true,
                            _ => knobs.ssao_samples > 0,
                        }
                });
            // The ray traced shadows need the same depth.
            let depth_prepass = match (
                ambient_occlusion.is_some() || ray_traced || ray_traced_lamps,
                hdr.view_target(),
            ) {
                (true, Some((target, width, height))) => {
                    let mut ssao = self.ssao.borrow_mut();
                    let ssao = ssao.get_or_insert_with(Ssao::new);
                    ssao.begin_depth_pass();
                    let depth_only = DepthPass::begin();
                    for entity in self.entities.iter() {
                        entity.render(self, &view_projection, parent_transform);
                    }
                    // Its full shape, trails and all, as the shading pass
                    // draws it.
                    self.render_snow_layer(&view_projection, false);
                    drop(depth_only);
                    ssao.take_depth(target, width, height);
                    match (ambient_occlusion, ray_traced_ao, ssao.depth_texture()) {
                        (Some(settings), true, Some(depth)) => {
                            if let Some(ray_traced) = self.ray_traced.borrow_mut().as_mut() {
                                ray_traced.trace_ao(
                                    &ShadowView {
                                        view_projection,
                                        eye: camera.get_camera().get_eye_position(),
                                        to_light: Vector3::unit_y(),
                                        depth,
                                        width,
                                        height,
                                    },
                                    &settings,
                                    SSAO_UNIT,
                                );
                                self.ambient_occlusion_origin.set(Some([0, 0]));
                            }
                        }
                        (Some(settings), _, _) => {
                            ssao.resolve(&camera.get_projection().get_matrix(), &settings);
                            ssao.bind_result();
                            self.ambient_occlusion_origin.set(Some([0, 0]));
                        }
                        _ => {}
                    }
                    if let (true, Some(depth), Some(lighting)) =
                        (ray_traced, ssao.depth_texture(), self.frame_lighting.get())
                    {
                        // From the moon at night, as the shadow map would be.
                        let to_light = if lighting.indirect.moon_shadows {
                            lighting.indirect.moon_direction
                        } else {
                            lighting.sun_direction
                        };
                        if let Some(ray_traced) = self.ray_traced.borrow_mut().as_mut() {
                            ray_traced.trace(&ShadowView {
                                view_projection,
                                eye: camera.get_camera().get_eye_position(),
                                to_light,
                                depth,
                                width,
                                height,
                            });
                            self.ray_traced_shadows_origin.set(Some([0, 0]));
                        }
                    }
                    if let (true, Some(depth)) = (ray_traced_lamps, ssao.depth_texture()) {
                        if let Some(ray_traced) = self.ray_traced.borrow_mut().as_mut() {
                            ray_traced.trace_lamps(
                                &ShadowView {
                                    view_projection,
                                    eye: camera.get_camera().get_eye_position(),
                                    to_light: Vector3::unit_y(),
                                    depth,
                                    width,
                                    height,
                                },
                                self.light_count.get(),
                            );
                            self.ray_traced_light_shadows_origin.set(Some([0, 0]));
                        }
                    }
                    true
                }
                _ => false,
            };

            // Sky pass — runs before opaque geometry so terrain/water overdraw
            // it naturally via the depth test. Depth write is disabled inside
            // the renderer so the sky never blocks scene geometry.
            profiler::mark(Pass::Sky);
            if let Some(skylight) = self.get_component::<SkyLight>() {
                let cam = camera.get_camera();
                let proj = camera.get_projection();
                let eye = cam.get_eye_position();
                let (sin_p, cos_p) = cam.get_pitch().0.sin_cos();
                let (sin_y, cos_y) = cam.get_yaw().0.sin_cos();
                let forward = Vector3::new(cos_p * cos_y, sin_p, cos_p * sin_y).normalize();
                let world_up = Vector3::unit_y();
                let right = forward.cross(world_up).normalize();
                let up = right.cross(forward).normalize();
                let tan_half_y = (proj.fovy.0 * 0.5).tan();
                let tan_half_x = tan_half_y * proj.aspect;
                SkyRenderer::render(
                    eye,
                    forward,
                    right,
                    up,
                    tan_half_x,
                    tan_half_y,
                    skylight.get_sun_direction(),
                    skylight.get_moon_direction(),
                    skylight.global_illumination().moonlight,
                    skylight.get_time_of_day(),
                    skylight.effective_cloud_coverage(),
                    skylight.effective_cloud_type(),
                    skylight.clouds(),
                    skylight.weather().precipitation().1,
                    // The opaque scene's depth, when it is drawn already.
                    depth_prepass
                        .then(|| self.ssao.borrow().as_ref().and_then(Ssao::depth_texture))
                        .flatten(),
                );
            }

            profiler::mark(Pass::Opaque);
            if let Some(shadow_fbo) = &self.shadow_fbo {
                shadow_fbo.texture().bind_to(SHADOW_MAP_UNIT);
            }
            if let Some(maps) = self.light_shadows.borrow().as_ref() {
                maps.bind();
            }
            if depth_prepass {
                gfx::set_pass_overrides(gfx::PassOverrides { depth_less_equal: true, ..gfx::pass_overrides() });
            }
            ViewFrustum::begin_counting();
            for entity in self.entities.iter() {
                entity.render(self, &view_projection, parent_transform);
            }
            ViewFrustum::end_counting();
            self.render_snow_layer(&view_projection, false);
            if depth_prepass {
                gfx::set_pass_overrides(gfx::PassOverrides { depth_less_equal: false, ..gfx::pass_overrides() });
            }
            // Transparent pass — runs after every entity's opaque geometry is
            // in the depth buffer, so water with depth-write enabled correctly
            // depth-tests against the player and other dynamic geometry. It
            // reads a copy of the opaque scene (what lies under the water).
            profiler::mark(Pass::Transparent);
            hdr.capture_opaque();
            for entity in self.entities.iter() {
                entity.render_transparent(self, &view_projection, parent_transform);
            }
            self.render_precipitation(&view_projection);
            self.render_frozen_view(&view_projection);
            ViewFrustum::end_view();
            self.ambient_occlusion_origin.set(None);
            self.ray_traced_shadows_origin.set(None);
        self.ray_traced_light_shadows_origin.set(None);
            profiler::mark(Pass::Post);
            hdr.finish(&hdr_settings);
            self.apply_viewport(window);
            self.render_debug_views(depth_prepass && ambient_occlusion.is_some());
            self.weather_maps.set(None);
            self.light_count.set(0);

            // 2D overlays (HUD, inventory, menus) on the tone mapped image.
            // In the order drawn: the plane and text renderers switch the
            // depth test on, so it passes everything and writes nothing.
            let before = gfx::pass_overrides();
            gfx::set_pass_overrides(gfx::PassOverrides { depth_ignored: true, ..before });
            for entity in self.entities.iter() {
                entity.render_overlay(self);
            }
            gfx::set_pass_overrides(before);
        }

        // ── Debug overlay (F10) ───────────────────────────────────────────
        if self.show_shadow_debug {
            // Retrieve the camera view-projection once.
            let vp = self.get_active_camera().map(|c| c.get_view_projection());

            // ── Terrain depth capture (camera POV, no water) ─────────────
            if let (Some(tfbo), Some(vp)) = (&self.terrain_debug_fbo, vp) {
                tfbo.0.begin_pass(gfx::Load::Color([0.0; 4]), gfx::Load::Depth(1.0));
                for entity in self.entities.iter() {
                    entity.render(self, &vp, parent_transform);
                }
                gfx::end_pass();
                self.apply_viewport(window);
            }

            // ── Water depth capture (camera POV, terrain depth + water) ──
            if let (Some(wfbo), Some(vp)) = (&self.water_debug_fbo, vp) {
                wfbo.0.begin_pass(gfx::Load::Color([0.0; 4]), gfx::Load::Depth(1.0));
                for entity in self.entities.iter() {
                    entity.render(self, &vp, parent_transform);
                }
                for entity in self.entities.iter() {
                    entity.render_transparent(self, &vp, parent_transform);
                }
                gfx::end_pass();
                self.apply_viewport(window);
            }

            // ── 2 × 2 panel layout on the right half of the screen ───────
            //
            //   x = 0.5 … 0.75
            //  ┌────────────────────┐  y = 0.5
            //  │  terrain depth     │
            //  │  (camera POV)      │
            //  ├────────────────────┤  y = 0.0
            //  │  water depth       │
            //  │  (camera POV)      │
            //  └────────────────────┘
            //
            if let Some(tfbo) = &self.terrain_debug_fbo {
                if let Some(tex) = tfbo.get_depth_texture() {
                    self.texture_renderer.render_depth(tex, 0.5, 0.5, 0.25, 0.5);
                }
            }
            if let Some(wfbo) = &self.water_debug_fbo {
                if let Some(tex) = wfbo.get_depth_texture() {
                    self.texture_renderer.render_depth(tex, 0.5, 0.0, 0.25, 0.5);
                }
            }
        }
    }

    /// The maps rain and snow are shaded with: the scene from above (what
    /// is under open sky) and the trails in the snow.
    fn render_weather_maps(&self, window: &Window) {
        self.weather_maps.set(None);
        let footprints = weather::take_footprints();
        let (Some(camera), Some(skylight)) =
            (self.get_active_camera(), self.get_component::<SkyLight>())
        else {
            return;
        };
        let weather = skylight.weather();
        if !weather.is_active() {
            return;
        }
        let eye = camera.get_camera().get_eye_position();
        let mut targets = self.weather.borrow_mut();
        let targets = targets.get_or_insert_with(WeatherTargets::new);
        if let Some(view_projection) = targets.exposure.begin(eye) {
            let _depth_only = DepthPass::begin();
            for entity in self.entities.iter() {
                entity.render(self, &view_projection, Matrix4::identity());
            }
            targets.exposure.end();
            if targets.exposure.ground_due() {
                let view_projection = targets.exposure.begin_ground();
                for entity in self.entities.iter() {
                    entity.render(self, &view_projection, Matrix4::identity());
                }
                targets.exposure.end();
            }
            if let Some((_, range)) = targets.exposure.projection() {
                targets.shelter.update(&targets.exposure, range);
            }
        }
        targets.exposure.bind();
        targets.shelter.bind();
        let trails = if weather.snow_cover > 0.0 {
            targets.trails.update(eye, weather.shading().snow, &footprints);
            targets.trails.bind();
            targets.trails.area()
        } else {
            None
        };
        targets.bind_snow_textures();
        self.weather_maps.set(Some(WeatherMaps {
            exposure: targets.exposure.projection(),
            trails,
            snow_textures: targets.snow_textures.is_some(),
        }));
        self.apply_viewport(window);
    }

    /// Bind the weather's maps to their units again (see
    /// `render_weather_maps`).
    fn bind_weather_maps(&self) {
        if self.weather_maps.get().is_none() {
            return;
        }
        if let Some(targets) = self.weather.borrow().as_ref() {
            targets.exposure.bind();
            targets.shelter.bind();
            targets.trails.bind();
            targets.bind_snow_textures();
        }
    }

    /// The layer of snow on the ground near the camera, with the opaque
    /// scene: in the shadow maps, the ambient occlusion's depth and the
    /// camera's view.
    /// `flat`: without the trails pressed in (shadows, ambient occlusion).
    fn render_snow_layer(&self, view_projection: &Matrix4<f32>, flat: bool) {
        let (Some(skylight), Some(lighting), Some(maps)) = (
            self.get_component::<SkyLight>(),
            Lighting::from_scene(self),
            self.weather_maps(),
        ) else {
            return;
        };
        let targets = self.weather.borrow();
        if let Some(targets) = targets.as_ref() {
            targets.layer.render(
                view_projection,
                lighting.camera_position,
                &lighting,
                &maps,
                skylight.effective_cloud_type(),
                flat,
            );
        }
    }

    /// Rain or snow falling around the camera, over the finished scene.
    fn render_precipitation(&self, view_projection: &Matrix4<f32>) {
        let Some(skylight) = self.get_component::<SkyLight>() else {
            return;
        };
        let (kind, amount) = skylight.weather().precipitation();
        let targets = self.weather.borrow();
        let (Some(targets), Some(lighting)) = (targets.as_ref(), Lighting::from_scene(self))
        else {
            return;
        };
        if amount > 0.0 {
            targets.precipitation.render(
                view_projection,
                lighting.camera_position,
                kind,
                amount,
                &lighting,
                self.weather_maps().as_ref(),
            );
        }
    }

    pub fn add_entity(&mut self, entity: Entity) {
        self.entities.push(entity);
    }

    /// Add `entity` as a child of `parent`, or at the root when `parent` is
    /// `None`. Returns the entity back if the parent does not exist.
    pub fn add_entity_to(
        &mut self,
        parent: Option<EntityHandle>,
        entity: Entity,
    ) -> Result<(), Entity> {
        match parent {
            None => {
                self.entities.push(entity);
                Ok(())
            }
            Some(parent) => match self.get_entity_mut(&parent) {
                Some(parent) => {
                    parent.add_child(entity);
                    Ok(())
                }
                None => Err(entity),
            },
        }
    }

    /// Detach an entity (with its children) from the scene without releasing
    /// its physics resources.
    pub fn take_entity(&mut self, id: &EntityHandle) -> Option<Entity> {
        if let Some(index) = self.entities.iter().position(|e| e.id == *id) {
            return Some(self.entities.remove(index));
        }
        self.entities
            .iter_mut()
            .find_map(|entity| entity.remove_descendant(id))
    }

    /// Remove an entity and its subtree, releasing physics bodies.
    pub fn remove_entity(&mut self, id: &EntityHandle) -> bool {
        match self.take_entity(id) {
            Some(mut entity) => {
                entity.release_resources(&mut self.physics_engine);
                true
            }
            None => false,
        }
    }

    pub fn get_entities_mut(&mut self) -> &mut Vec<Entity> {
        &mut self.entities
    }

    /// Ids of the ancestors of `id`, root first. `None` if `id` is unknown.
    pub fn get_ancestors(&self, id: &EntityHandle) -> Option<Vec<EntityHandle>> {
        fn walk(entity: &Entity, id: &EntityHandle, path: &mut Vec<EntityHandle>) -> bool {
            if entity.id == *id {
                return true;
            }
            path.push(entity.id);
            for child in entity.get_children() {
                if walk(child, id, path) {
                    return true;
                }
            }
            path.pop();
            false
        }
        for entity in self.entities.iter() {
            let mut path = Vec::new();
            if walk(entity, id, &mut path) {
                return Some(path);
            }
        }
        None
    }

    /// World transform of the parent of `id` (identity for root entities).
    pub fn get_parent_world_transform(&self, id: &EntityHandle) -> Option<Matrix4<f32>> {
        let ancestors = self.get_ancestors(id)?;
        let mut transform = Matrix4::identity();
        for ancestor in ancestors {
            transform = transform * self.get_entity(&ancestor)?.local_transform();
        }
        Some(transform)
    }

    pub fn get_world_transform(&self, id: &EntityHandle) -> Option<Matrix4<f32>> {
        let parent = self.get_parent_world_transform(id)?;
        Some(parent * self.get_entity(id)?.local_transform())
    }

    /// Closest persistent entity whose physics shape the ray hits, with the
    /// hit distance. Hits on transient entities (terrain chunks) resolve to
    /// their nearest persistent ancestor (the terrain).
    pub fn raycast_entity(
        &mut self,
        origin: Point3<f32>,
        dir: Vector3<f32>,
        max_distance: f32,
    ) -> Option<(EntityHandle, f32)> {
        let (body, distance) = self.physics_engine.cast_ray_bodies(
            [origin.x, origin.y, origin.z],
            [dir.x, dir.y, dir.z],
            max_distance,
        )?;
        fn find(
            entity: &Entity,
            owner: Option<EntityHandle>,
            body: rapier3d::prelude::RigidBodyHandle,
        ) -> Option<EntityHandle> {
            let owner = if entity.is_transient() {
                owner
            } else {
                Some(entity.id)
            };
            if entity
                .get_own_component::<RigidBody>()
                .is_some_and(|rb| rb.rigid_body_handle == body)
            {
                return owner;
            }
            entity
                .get_children()
                .iter()
                .find_map(|child| find(child, owner, body))
        }
        self.entities
            .iter()
            .find_map(|entity| find(entity, None, body))
            .map(|id| (id, distance))
    }

    /// Nearest entity whose components' geometry the ray (`dir` normalized)
    /// hits, with the distance (see `Component::raycast`). Finds what has no
    /// physics shape, such as plain models. A hit on a transient entity
    /// counts for its nearest persistent ancestor.
    pub fn raycast_components(
        &self,
        origin: Point3<f32>,
        dir: Vector3<f32>,
    ) -> Option<(EntityHandle, f32)> {
        fn visit(
            entity: &Entity,
            parent: Matrix4<f32>,
            owner: Option<EntityHandle>,
            origin: Point3<f32>,
            dir: Vector3<f32>,
            nearest: &mut Option<(EntityHandle, f32)>,
        ) {
            let owner = if entity.is_transient() {
                owner
            } else {
                Some(entity.id)
            };
            let world = parent * entity.local_transform();
            if let Some(owner) = owner {
                for component in entity.components() {
                    let Some(t) = component.raycast(&world, origin, dir) else {
                        continue;
                    };
                    if nearest.map_or(true, |(_, n)| t < n) {
                        *nearest = Some((owner, t));
                    }
                }
            }
            for child in entity.get_children() {
                visit(child, world, owner, origin, dir, nearest);
            }
        }
        let mut nearest = None;
        for entity in &self.entities {
            visit(entity, Matrix4::identity(), None, origin, dir, &mut nearest);
        }
        nearest
    }

    /// Wireframes of every collider whose component has its "show
    /// collider" toggle on: `(primitive shapes, triangle meshes)`. Meshes are
    /// limited to `mesh_distance` around `camera`.
    pub fn visible_collider_lines(
        &self,
        camera: Point3<f32>,
        mesh_distance: f32,
    ) -> (Vec<Line>, Vec<Line>) {
        fn visit(entity: &Entity, bodies: &mut Vec<rapier3d::prelude::RigidBodyHandle>) {
            for component in entity.components() {
                bodies.extend(component.visible_collider_bodies(entity));
            }
            for child in entity.get_children() {
                visit(child, bodies);
            }
        }
        let mut bodies = Vec::new();
        for entity in self.entities.iter() {
            visit(entity, &mut bodies);
        }
        let (mut shapes, mut meshes) = (Vec::new(), Vec::new());
        collider_debug::body_wires(
            &self.physics_engine,
            &bodies,
            camera,
            mesh_distance,
            &mut shapes,
            &mut meshes,
        );
        (shapes, meshes)
    }

    /// Move an entity (local space) and any rigid body attached to it.
    pub fn set_entity_position(&mut self, id: &EntityHandle, position: Point3<f32>) {
        let Some(entity) = self.get_entity_mut(id) else {
            return;
        };
        entity.set_local_position(position);
        let handle = entity
            .get_own_component::<RigidBody>()
            .map(|rb| rb.rigid_body_handle);
        if let Some(handle) = handle {
            if let Some(body) = self.physics_engine.rigid_bodies.get_mut(handle) {
                body.set_translation(
                    rapier3d::prelude::Vector::new(position.x, position.y, position.z),
                    true,
                );
                self.physics_engine.body_moved();
            }
        }
    }

    pub fn set_entity_rotation(&mut self, id: &EntityHandle, rotation: Quaternion<f32>) {
        if let Some(entity) = self.get_entity_mut(id) {
            entity.set_local_rotation(rotation);
        }
    }

    pub fn set_entity_scale(&mut self, id: &EntityHandle, scale: Vector3<f32>) {
        if let Some(entity) = self.get_entity_mut(id) {
            entity.set_local_scale(scale);
        }
    }

    pub fn handle_event(
        &mut self,
        glfw: &mut Glfw,
        window: &mut glfw::Window,
        event: &WindowEvent,
    ) {
        if let WindowEvent::Key(Key::F10, _, Action::Press, _) = event {
            self.show_shadow_debug = !self.show_shadow_debug;
        }
        // Rendered into a part of the window (the editor's viewport), the
        // cursor is given relative to that part. Only a visible cursor: a
        // captured one (mouse look) moves from wherever it was reset to.
        let local;
        let event = match (self.viewport, event) {
            (Some([x, y, _, h]), WindowEvent::CursorPos(cx, cy))
                if window.get_cursor_mode() == glfw::CursorMode::Normal =>
            {
                let top = window.get_framebuffer_size().1 - (y + h);
                local = WindowEvent::CursorPos(cx - x as f64, cy - top as f64);
                &local
            }
            _ => event,
        };
        for entity in self.entities.iter_mut() {
            entity.handle_event(glfw, window, event);
        }
    }

    pub fn get_component<T>(&self) -> Option<&T>
    where
        T: Component,
    {
        for entity in self.entities.iter() {
            if let Some(component) = entity.get_component::<T>() {
                return Some(component);
            }
        }
        None
    }

    pub fn get_component_mut<T>(&mut self) -> Option<&mut T>
    where
        T: Component,
    {
        for entity in self.entities.iter_mut() {
            if let Some(component) = entity.get_component_mut::<T>() {
                return Some(component);
            }
        }
        None
    }

    // pub fn get_components<T>(&self) -> Vec<&T>
    // where
    //     T: Component,
    // {
    //     let mut components = Vec::new();
    //     for entity in self.entities.iter() {
    //         if let Some(component) = entity.get_component::<T>() {
    //             components.push(component);
    //         }
    //     }
    //     components
    // }

    pub fn get_entities_with_component<T>(&self) -> Vec<&Entity>
    where
        T: Component,
    {
        let mut entities = Vec::new();
        for entity in self.entities.iter() {
            entities.extend(entity.get_with_own_component::<T>());
        }
        entities
    }

    pub fn get_entities(&self) -> &Vec<Entity> {
        &self.entities
    }

    pub fn get_entity(&self, id: &EntityHandle) -> Option<&Entity> {
        for entity in self.entities.iter() {
            if entity.id == *id {
                return Some(entity);
            }
            if let Some(entity) = entity.get_child(id) {
                return Some(entity);
            }
        }
        None
    }

    pub fn get_entity_mut(&mut self, id: &EntityHandle) -> Option<&mut Entity> {
        for entity in self.entities.iter_mut() {
            if entity.id == *id {
                return Some(entity);
            }
            if let Some(entity) = entity.get_child_mut(id) {
                return Some(entity);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene_with(names: &[&str]) -> (Scene, Vec<EntityHandle>) {
        crate::core::headless::enable();
        let mut scene = Scene::new();
        let ids = names
            .iter()
            .map(|name| {
                let entity = Entity::new(name);
                let id = entity.id;
                scene.add_entity(entity);
                id
            })
            .collect();
        (scene, ids)
    }

    fn names(scene: &Scene) -> Vec<String> {
        scene.get_entities().iter().map(Entity::get_name).collect()
    }

    #[test]
    fn an_update_that_removes_an_earlier_root_skips_none_and_does_not_panic() {
        let (mut scene, ids) = scene_with(&["a", "b", "c", "d"]);
        let mut updated = Vec::new();
        scene.update_roots(|entity, scene| {
            updated.push(entity.get_name());
            if entity.id == ids[1] {
                assert!(scene.remove_entity(&ids[0]));
            }
        });
        assert_eq!(updated, ["a", "b", "c", "d"]);
        assert_eq!(names(&scene), ["b", "c", "d"]);
    }

    #[test]
    fn an_update_that_removes_a_later_root_does_not_panic() {
        let (mut scene, ids) = scene_with(&["a", "b", "c"]);
        let mut updated = Vec::new();
        scene.update_roots(|entity, scene| {
            updated.push(entity.get_name());
            if entity.id == ids[0] {
                assert!(scene.remove_entity(&ids[2]));
            }
        });
        assert_eq!(updated, ["a", "b"]);
        assert_eq!(names(&scene), ["a", "b"]);
    }

    #[test]
    fn an_update_that_replaces_the_scene_leaves_the_new_one_alone() {
        let (mut scene, _) = scene_with(&["a", "b"]);
        let mut updated = Vec::new();
        scene.update_roots(|entity, scene| {
            updated.push(entity.get_name());
            let (other, _) = scene_with(&["x"]);
            scene.replace_with(other);
        });
        assert_eq!(updated, ["a"]);
        assert_eq!(names(&scene), ["x"]);
    }

    #[test]
    fn roots_are_updated_in_order_and_stay_in_order() {
        let (mut scene, ids) = scene_with(&["a", "b", "c", "d"]);
        let mut updated = Vec::new();
        scene.update_roots(|entity, scene| {
            updated.push(entity.get_name());
            // Out of the scene while it runs.
            assert!(scene.get_entity(&entity.id).is_none());
            // What stands in for it is neither saved nor found.
            assert_eq!(scene.to_file().entities.len(), 3);
            assert!(scene.find_entity(&entity.get_name()).is_none());
        });
        assert_eq!(updated, ["a", "b", "c", "d"]);
        assert_eq!(names(&scene), ["a", "b", "c", "d"]);
        let after: Vec<_> = scene.get_entities().iter().map(|e| e.id).collect();
        assert_eq!(after, ids);
    }

    #[test]
    fn an_update_that_adds_roots_updates_them_after_the_others() {
        let (mut scene, _) = scene_with(&["a", "b"]);
        let mut updated = Vec::new();
        scene.update_roots(|entity, scene| {
            updated.push(entity.get_name());
            if entity.get_name() == "a" {
                scene.add_entity(Entity::new("new"));
            }
        });
        assert_eq!(updated, ["a", "b", "new"]);
        assert_eq!(names(&scene), ["a", "b", "new"]);
    }

    #[test]
    fn an_update_that_moves_a_later_root_to_the_end_skips_none() {
        let (mut scene, ids) = scene_with(&["a", "b", "c", "d"]);
        let mut updated = Vec::new();
        scene.update_roots(|entity, scene| {
            updated.push(entity.get_name());
            if entity.id == ids[0] {
                let b = scene.take_entity(&ids[1]).unwrap();
                scene.add_entity(b);
            }
        });
        assert_eq!(updated, ["a", "c", "d", "b"]);
        assert_eq!(names(&scene), ["a", "c", "d", "b"]);
    }

    #[test]
    fn an_update_that_sweeps_out_its_own_place_puts_the_entity_back() {
        let (mut scene, _) = scene_with(&["a", "b", "c"]);
        let mut updated = Vec::new();
        scene.update_roots(|entity, scene| {
            updated.push(entity.get_name());
            if entity.get_name() == "a" {
                // Drops everything transient at the root, the place of `a`
                // among them.
                scene.get_entities_mut().retain(|e| !e.is_transient());
            }
        });
        assert_eq!(updated, ["a", "b", "c"]);
        assert_eq!(names(&scene), ["a", "b", "c"]);
    }
}

/// Spike probe (see `core::spike_probe`): adds its lifetime to a counter.
struct Probe(std::time::Instant, &'static std::sync::atomic::AtomicU64);
impl Drop for Probe {
    fn drop(&mut self) {
        crate::core::spike_probe::add(self.1, self.0);
    }
}
