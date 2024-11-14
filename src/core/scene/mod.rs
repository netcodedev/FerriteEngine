use super::{
    entity::Entity,
    renderer::{framebuffer::ShadowFrameBuffer, texture::TextureRenderer},
};

mod scene;

pub struct Scene {
    entities: Vec<Entity>,

    shadow_fbo: Option<ShadowFrameBuffer>,

    texture_renderer: TextureRenderer,
}
