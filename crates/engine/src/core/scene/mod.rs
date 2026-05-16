use super::{
    entity::Entity,
    physics::physics_engine::PhysicsEngine,
    renderer::{framebuffer::ShadowFrameBuffer, texture::TextureRenderer},
};

mod scene;

pub struct Scene {
    entities: Vec<Entity>,
    pub physics_engine: PhysicsEngine,
    shadow_fbo: Option<ShadowFrameBuffer>,
    /// Camera-perspective depth capture — terrain only, no water.
    terrain_debug_fbo: Option<ShadowFrameBuffer>,
    /// Camera-perspective depth+colour capture — terrain depth + water on top.
    water_debug_fbo: Option<ShadowFrameBuffer>,
    texture_renderer: TextureRenderer,
    show_shadow_debug: bool,
}
