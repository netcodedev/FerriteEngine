use cgmath::{InnerSpace, Matrix4, Point3, Quaternion, SquareMatrix, Vector3};
use glfw::{Action, Glfw, Key, WindowEvent};

use crate::core::{
    entity::{
        component::{camera_component::CameraComponent, Component},
        Entity, EntityHandle,
    },
    physics::{collider_debug, physics_engine::PhysicsEngine, rigidbody::RigidBody},
    renderer::{
        framebuffer::{FrameBuffer, ShadowFrameBuffer},
        line::Line,
        light::skylight::SkyLight,
        sky::SkyRenderer,
        texture::TextureRenderer,
    },
    window::Window,
};

use super::Scene;

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
        }
    }

    pub fn add_shadow_map(&mut self, width: u32, height: u32) {
        self.shadow_fbo = Some(ShadowFrameBuffer::new(width, height));
        self.shadow_map_size = Some((width, height));
    }

    /// Create the camera-perspective debug FBOs used by F10.
    /// `width`/`height` should match the window dimensions.
    pub fn add_debug_maps(&mut self, width: u32, height: u32) {
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

    fn apply_viewport(&self, window: &Window) {
        match self.viewport {
            Some([x, y, w, h]) => unsafe { gl::Viewport(x, y, w.max(1), h.max(1)) },
            None => window.reset_viewport(),
        }
    }

    /// Edit-mode update: no physics, no gameplay — only components that keep
    /// the scene presentable (see `Component::update_in_editor`).
    pub fn update_in_editor(&mut self, delta_time: f64) {
        for i in 0..self.entities.len() {
            let mut entity = self.entities.remove(i);
            entity.update_in_editor(self, delta_time);
            self.entities.insert(i, entity);
        }
    }

    /// Returns whether the currently-bound framebuffer allows water rendering.
    /// Water is allowed on the default framebuffer (FBO 0) and on the
    /// dedicated water-debug FBO only.  All other FBOs (shadow map,
    /// terrain-debug) should skip the transparent water pass.
    pub fn is_water_render_allowed(&self) -> bool {
        let mut bound: gl::types::GLint = 0;
        unsafe { gl::GetIntegerv(gl::DRAW_FRAMEBUFFER_BINDING, &mut bound); }
        let bound = bound as u32;
        if bound == 0 {
            return true;
        }
        if let Some(wfbo) = &self.water_debug_fbo {
            if bound == wfbo.get_id() {
                return true;
            }
        }
        false
    }

    /// Returns true when rendering into the water depth-capture FBO.
    /// In this mode `render_transparent` writes depth (DepthMask=TRUE,
    /// GL_ALWAYS) instead of blending colour, so the depth texture shows
    /// exactly where the water surface sits in clip-space.
    pub fn is_water_depth_capture(&self) -> bool {
        let mut bound: gl::types::GLint = 0;
        unsafe { gl::GetIntegerv(gl::DRAW_FRAMEBUFFER_BINDING, &mut bound); }
        let bound = bound as u32;
        self.water_debug_fbo
            .as_ref()
            .map_or(false, |fbo| fbo.get_id() == bound)
    }

    pub fn update(&mut self, delta_time: f64) {
        if self.physics_enabled {
            self.physics_engine.update((delta_time as f32).min(1.0 / 20.0));
        }
        for i in 0..self.entities.len() {
            let mut entity = self.entities.remove(i);
            entity.update(self, delta_time);
            self.entities.insert(i, entity);
        }
    }

    pub fn render(&self, window: &Window) {
        let parent_transform = Matrix4::identity();

        // Shadow Pass
        if let Some(shadow_fbo) = &self.shadow_fbo {
            if let Some(skylight) = self.get_component::<SkyLight>() {
                let light_projection = skylight.get_projection();
                unsafe {
                    gl::ActiveTexture(gl::TEXTURE15);
                    gl::BindTexture(gl::TEXTURE_2D, 0); // Unbind to prevent read/write feedback loop
                }
                shadow_fbo.bind();
                window.clear_mask(gl::DEPTH_BUFFER_BIT | gl::COLOR_BUFFER_BIT);
                for entity in self.entities.iter() {
                    entity.render(self, &light_projection, parent_transform);
                }
                FrameBuffer::unbind();
                self.apply_viewport(window);
            }
        }

        // Render Pass
        self.apply_viewport(window);
        if let Some(camera) = self.get_active_camera() {
            let view_projection = camera.get_view_projection();

            // Sky pass — runs before opaque geometry so terrain/water overdraw
            // it naturally via the depth test. Depth write is disabled inside
            // the renderer so the sky never blocks scene geometry.
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
                    skylight.get_time_of_day(),
                    skylight.get_cloud_coverage(),
                );
            }

            if let Some(shadow_fbo) = &self.shadow_fbo {
                if let Some(texture) = &shadow_fbo.get_depth_texture() {
                    unsafe {
                        gl::ActiveTexture(gl::TEXTURE15);
                    }
                    texture.bind();
                }
            }
            for entity in self.entities.iter() {
                entity.render(self, &view_projection, parent_transform);
            }
            // Transparent pass — runs after every entity's opaque geometry is
            // in the depth buffer, so water with depth-write enabled correctly
            // depth-tests against the player and other dynamic geometry.
            for entity in self.entities.iter() {
                entity.render_transparent(self, &view_projection, parent_transform);
            }
        }

        // ── Debug overlay (F10) ───────────────────────────────────────────
        if self.show_shadow_debug {
            // Retrieve the camera view-projection once.
            let vp = self.get_active_camera().map(|c| c.get_view_projection());

            // ── Terrain depth capture (camera POV, no water) ─────────────
            if let (Some(tfbo), Some(vp)) = (&self.terrain_debug_fbo, vp) {
                tfbo.bind();
                window.clear_mask(gl::DEPTH_BUFFER_BIT | gl::COLOR_BUFFER_BIT);
                for entity in self.entities.iter() {
                    entity.render(self, &vp, parent_transform);
                }
                FrameBuffer::unbind();
                self.apply_viewport(window);
            }

            // ── Water depth capture (camera POV, terrain depth + water) ──
            if let (Some(wfbo), Some(vp)) = (&self.water_debug_fbo, vp) {
                wfbo.bind();
                window.clear_mask(gl::DEPTH_BUFFER_BIT | gl::COLOR_BUFFER_BIT);
                for entity in self.entities.iter() {
                    entity.render(self, &vp, parent_transform);
                }
                for entity in self.entities.iter() {
                    entity.render_transparent(self, &vp, parent_transform);
                }
                FrameBuffer::unbind();
                self.apply_viewport(window);
            }

            // ── 2 × 2 panel layout on the right half of the screen ───────
            //
            //   x = 0.5 … 0.75           x = 0.75 … 1.0
            //  ┌────────────────────┬────────────────────┐  y = 0.5
            //  │  terrain depth     │  shadow depth      │
            //  │  (camera POV)      │  (light POV)       │
            //  ├────────────────────┼────────────────────┤  y = 0.0
            //  │  water depth       │  shadow colour     │
            //  │  (camera POV)      │                    │
            //  └────────────────────┴────────────────────┘
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
            if let Some(sfbo) = &self.shadow_fbo {
                if let Some(tex) = sfbo.get_depth_texture() {
                    self.texture_renderer.render_depth(tex, 0.75, 0.5, 0.25, 0.5);
                }
                if let Some(tex) = sfbo.get_color_texture() {
                    self.texture_renderer.render_color(tex, 0.75, 0.0, 0.25, 0.5);
                }
            }
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
