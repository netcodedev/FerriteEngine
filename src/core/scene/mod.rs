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
    texture_renderer: TextureRenderer,
}
