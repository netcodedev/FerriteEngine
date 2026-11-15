use std::cell::{Cell, RefCell};

use super::{
    entity::{component::camera_component::CameraComponent, Entity},
    physics::physics_engine::PhysicsEngine,
    renderer::{
        framebuffer::{CascadedShadowMap, ShadowFrameBuffer},
        hdr::Hdr,
        pbr::Lighting,
        light::{
            light::{LightBuffer, LightSource},
            shadow::{LightShadowMaps, ShadowPlanner},
        },
        ssao::Ssao,
        texture::TextureRenderer,
        weather::{WeatherMaps, WeatherTargets},
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
    /// Ambient occlusion targets, made on the first frame that uses them.
    ssao: RefCell<Option<Ssao>>,
    /// The camera's HDR target and post passes, made on the first frame.
    hdr: RefCell<Option<Hdr>>,
    /// Window position of the ambient occlusion while the camera's view is
    /// shaded with it, `None` otherwise (see `pbr::Lighting`).
    ambient_occlusion_origin: Cell<Option<[i32; 2]>>,
    /// Rain and snow: the sky exposure map, the snow trails and the falling
    /// drops, made on the first frame with weather.
    weather: RefCell<Option<WeatherTargets>>,
    /// Where the weather's maps lie while the scene is shaded with them.
    weather_maps: Cell<Option<WeatherMaps>>,
    /// The scene's lights for the shaders, made on the first frame with one.
    lights: RefCell<Option<LightBuffer>>,
    /// How many lights the buffer holds while the scene is drawn.
    light_count: Cell<usize>,
    /// The lights in the buffer this frame, with their shadow layers.
    light_sources: RefCell<Vec<LightSource>>,
    /// Shadow cube maps of point and spot lights, made when first needed.
    light_shadows: RefCell<Option<LightShadowMaps>>,
    /// Which of their faces are drawn this frame.
    shadow_planner: RefCell<ShadowPlanner>,
    /// The sun, sky and camera part of `Lighting` while the scene is drawn,
    /// looked up once per frame instead of on every draw (see
    /// `Lighting::from_scene`).
    frame_lighting: Cell<Option<Lighting>>,
    /// Work queued with `Scene::defer`, run once every entity is updated.
    deferred: Vec<Box<dyn FnOnce(&mut Scene)>>,
}
