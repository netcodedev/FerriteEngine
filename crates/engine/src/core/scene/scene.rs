use cgmath::{Matrix4, SquareMatrix};
use glfw::{Action, Glfw, Key, WindowEvent};

use crate::core::{
    entity::{
        component::{camera_component::CameraComponent, Component},
        Entity, EntityHandle,
    },
    physics::physics_engine::PhysicsEngine,
    renderer::{
        framebuffer::{FrameBuffer, ShadowFrameBuffer},
        light::skylight::SkyLight,
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
        }
    }

    pub fn add_shadow_map(&mut self, width: u32, height: u32) {
        self.shadow_fbo = Some(ShadowFrameBuffer::new(width, height));
    }

    /// Create the camera-perspective debug FBOs used by F10.
    /// `width`/`height` should match the window dimensions.
    pub fn add_debug_maps(&mut self, width: u32, height: u32) {
        self.terrain_debug_fbo = Some(ShadowFrameBuffer::new(width, height));
        self.water_debug_fbo   = Some(ShadowFrameBuffer::new(width, height));
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
        self.physics_engine.update((delta_time as f32).min(1.0 / 20.0));
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
                window.reset_viewport();
            }
        }

        // Render Pass
        if let Some(camera) = self.get_component::<CameraComponent>() {
            let view_projection = camera.get_view_projection();
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
            let vp = self
                .get_component::<CameraComponent>()
                .map(|c| c.get_view_projection());

            // ── Terrain depth capture (camera POV, no water) ─────────────
            if let (Some(tfbo), Some(vp)) = (&self.terrain_debug_fbo, vp) {
                tfbo.bind();
                window.clear_mask(gl::DEPTH_BUFFER_BIT | gl::COLOR_BUFFER_BIT);
                for entity in self.entities.iter() {
                    entity.render(self, &vp, parent_transform);
                }
                FrameBuffer::unbind();
                window.reset_viewport();
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
                window.reset_viewport();
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
