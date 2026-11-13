use super::{
    entity::{component::camera_component::CameraComponent, Entity},
    physics::physics_engine::PhysicsEngine,
    renderer::{
        framebuffer::{CascadedShadowMap, ShadowFrameBuffer},
        texture::TextureRenderer,
    },
};

mod scene;
pub mod scene_file;

pub struct Scene {
    entities: Vec<Entity>,
    pub physics_engine: PhysicsEngine,
    shadow_fbo: Option<CascadedShadowMap>,
    /// Camera-perspective depth capture — terrain only, no water.
    terrain_debug_fbo: Option<ShadowFrameBuffer>,
    /// Camera-perspective depth+colour capture — terrain depth + water on top.
    water_debug_fbo: Option<ShadowFrameBuffer>,
    texture_renderer: TextureRenderer,
    show_shadow_debug: bool,

    /// Sizes the render targets were created with, kept for scene files.
    shadow_map_size: Option<(u32, u32)>,
    debug_map_size: Option<(u32, u32)>,

    /// When set, rendering, shadows and terrain follow this camera instead of
    /// the scene's own `CameraComponent` (used by the editor in edit mode).
    editor_camera: Option<CameraComponent>,
    physics_enabled: bool,
    /// GL viewport `[x, y, width, height]` (bottom-left origin) for the main
    /// render pass. `None` uses the whole window.
    viewport: Option<[i32; 4]>,
    /// Work queued with `Scene::defer`, run once every entity is updated.
    deferred: Vec<Box<dyn FnOnce(&mut Scene)>>,
}
