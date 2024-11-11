use super::{
    entity::Entity,
    renderer::{framebuffer::FrameBuffer, texture::TextureRenderer},
};

mod scene;

pub struct Scene {
    entities: Vec<Entity>,

    shadow_fbo: Option<FrameBuffer>,

    texture_renderer: TextureRenderer,
}
