use std::cell::{Cell, RefCell};

use super::{
    entity::{component::camera_component::CameraComponent, Entity},
    physics::physics_engine::PhysicsEngine,
    renderer::{
        framebuffer::{CascadedShadowMap, ShadowFrameBuffer},
        hdr::Hdr,
        light::skylight::{ShadowCascade, SHADOW_CASCADES},
        pbr::Lighting,
        raytracing::RayTracedShadows,
        light::{
            clusters::ClusterBuffers,
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
    /// Ray traced sun shadows (graphics setting), made on the first frame
    /// that uses them.
    ray_traced: RefCell<Option<RayTracedShadows>>,
    /// Window position of the ray traced shadows while the camera's view
    /// is shaded with them, `None` otherwise (see `pbr::Lighting`).
    ray_traced_shadows_origin: Cell<Option<[i32; 2]>>,
    /// Window position of the ray traced lamp shadows while the camera's
    /// view is shaded with them.
    ray_traced_light_shadows_origin: Cell<Option<[i32; 2]>>,
    /// Rain and snow: the sky exposure map, the snow trails and the falling
    /// drops, made on the first frame with weather.
    weather: RefCell<Option<WeatherTargets>>,
    /// Where the weather's maps lie while the scene is shaded with them.
    weather_maps: Cell<Option<WeatherMaps>>,
    /// The scene's lights for the shaders, made on the first frame with one.
    lights: RefCell<Option<LightBuffer>>,
    /// Which lights reach which part of the camera's view (see
    /// `light::clusters`), built with the lights.
    light_clusters: RefCell<Option<ClusterBuffers>>,
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
    /// The sun's shadow cascades as their layers were last drawn (the far
    /// ones are not drawn every frame), `None` to draw them all.
    drawn_cascades: Cell<Option<[ShadowCascade; SHADOW_CASCADES]>>,
    /// Frames drawn with the sun's shadows, to stagger the far cascades.
    shadow_frame: Cell<u64>,
    /// Whether what casts the sun's shadows changed shape since the far
    /// cascades were drawn (see `Scene::request_shadow_redraw`): then they
    /// are all drawn again this frame.
    shadow_redraw: Cell<bool>,
    /// Where the camera was at the last shadow pass.
    shadow_eye: Cell<Option<cgmath::Point3<f32>>>,
    /// Work queued with `Scene::defer`, run once every entity is updated.
    deferred: Vec<Box<dyn FnOnce(&mut Scene)>>,
    /// Different for every scene made, so that the update loop sees when
    /// an entity's update replaced the whole scene (a level was loaded).
    generation: u64,
}
