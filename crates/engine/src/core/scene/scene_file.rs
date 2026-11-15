//! Serializable scene description (RON on disk) and the registry that turns
//! it back into live entities and components.

use std::{error::Error, fs, path::Path};

use cgmath::{Deg, Point3, Quaternion};
use serde::{Deserialize, Serialize};

use crate::{
    core::{
        assets::AssetRef,
        camera::{Camera, CameraController, Projection},
        entity::{
            component::{
                animation_component::AnimationComponent, camera_component::CameraComponent,
                debug_component::DebugController, missing_component::MissingComponent,
                model_component::ModelComponent,
                shape_component::{ShapeComponent, ShapeKind},
            },
            Entity, EntityHandle,
        },
        renderer::{
            hdr::HdrSettings,
            light::{
                global_illumination::GlobalIllumination,
                light::{Light, LightKind, LightSettings},
                skylight::SkyLight,
            },
            sky::CloudSettings,
            ssao::AmbientOcclusionSettings, weather::Weather,
        },
        script,
    },
    player::{build_animation_graph, default_animation_graph, load_graph_file, Player},
    terrain::{
        dual_contouring::DualContouringChunk,
        scatter::{Scatter, ScatterLayerData},
        Terrain,
    },
};

use super::Scene;

pub const SCENE_FILE_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SceneFile {
    pub version: u32,
    #[serde(default)]
    pub settings: SceneSettings,
    #[serde(default)]
    pub entities: Vec<EntityData>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct SceneSettings {
    #[serde(default)]
    pub shadow_map_size: Option<(u32, u32)>,
    #[serde(default)]
    pub debug_map_size: Option<(u32, u32)>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EntityData {
    pub id: u64,
    pub name: String,
    pub position: [f32; 3],
    /// Quaternion as `[w, x, y, z]`.
    pub rotation: [f32; 4],
    /// Per local axis; left out of the file when it is 1.
    #[serde(default = "unit_scale", skip_serializing_if = "is_unit_scale")]
    pub scale: [f32; 3],
    #[serde(default)]
    pub components: Vec<ComponentData>,
    #[serde(default)]
    pub children: Vec<EntityData>,
}

/// Every component kind that can be saved. Add a variant here (plus
/// `Component::to_data` and an arm in `attach`) to make a component
/// persistable.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ComponentData {
    Camera {
        position: [f32; 3],
        yaw_deg: f32,
        pitch_deg: f32,
        fov_deg: f32,
        near: f32,
        far: f32,
        speed: f32,
        /// Flies with WASD and the mouse when nothing else (a player
        /// controller) drives it. Missing in older scenes: off.
        #[serde(default)]
        free: bool,
    },
    SkyLight {
        time_of_day: f32,
        day_length_seconds: f32,
        paused: bool,
        cloud_coverage: f32,
        /// Missing in scenes saved before it existed: the defaults.
        #[serde(default)]
        clouds: CloudSettings,
        /// Missing in scenes saved before it existed: on, with defaults.
        #[serde(default)]
        ambient_occlusion: AmbientOcclusionSettings,
        /// Missing in scenes saved before it existed: the defaults.
        #[serde(default)]
        hdr: HdrSettings,
        /// Missing in scenes saved before it existed: clear and dry.
        #[serde(default)]
        weather: Weather,
        /// Moonlight, night sky light and lamp light bounce. Missing in
        /// scenes saved before it existed: the defaults.
        #[serde(default)]
        global_illumination: GlobalIllumination,
    },
    /// A point, spot or directional light in any colour (sRGB, 0 to 1).
    /// Every field may be left out for its default (see `LightSettings`).
    Light {
        #[serde(default)]
        kind: LightKind,
        #[serde(default = "light_default::color")]
        color: [f32; 3],
        #[serde(default = "light_default::intensity")]
        intensity: f32,
        #[serde(default = "light_default::range")]
        range: f32,
        #[serde(default = "light_default::inner_angle")]
        inner_angle: f32,
        #[serde(default = "light_default::outer_angle")]
        outer_angle: f32,
        #[serde(default = "light_default::bounce")]
        bounce: f32,
        #[serde(default = "light_default::enabled")]
        enabled: bool,
        #[serde(default = "light_default::shadows")]
        shadows: bool,
    },
    Terrain {
        seed: u64,
        /// Models scattered over the terrain, each with its own density.
        #[serde(default)]
        scatter: Vec<ScatterLayerData>,
    },
    DebugController,
    /// Prefab marker: the entity is rebuilt with `Player::new`, which brings
    /// its model, animation graph, physics capsule and bone colliders. What
    /// moves it is a script of the game.
    Player,
    /// Animation graph file of the entity's animation component (replaces
    /// the graph a prefab created).
    AnimationGraph {
        path: String,
    },
    /// The model asset the entity shows (for a prefab: instead of the one
    /// it comes with).
    Model {
        asset: AssetRef,
    },
    /// A cube, sphere or cylinder in one colour (sRGB, 0 to 1), one metre
    /// across. Needs no asset.
    Shape {
        kind: ShapeKind,
        color: [f32; 3],
    },
    /// A component written in a game crate (see `core::script`), by name,
    /// with the data it is built from.
    Script {
        name: String,
        #[serde(default = "unit_value")]
        data: ron::Value,
    },
}

fn unit_value() -> ron::Value {
    ron::Value::Unit
}

/// Defaults of the `Light` fields left out of a scene file.
mod light_default {
    use super::LightSettings;

    pub fn color() -> [f32; 3] {
        LightSettings::default().color
    }
    pub fn intensity() -> f32 {
        LightSettings::default().intensity
    }
    pub fn range() -> f32 {
        LightSettings::default().range
    }
    pub fn inner_angle() -> f32 {
        LightSettings::default().inner_angle
    }
    pub fn outer_angle() -> f32 {
        LightSettings::default().outer_angle
    }
    pub fn bounce() -> f32 {
        LightSettings::default().bounce
    }
    pub fn enabled() -> bool {
        LightSettings::default().enabled
    }
    pub fn shadows() -> bool {
        LightSettings::default().shadows
    }
}

impl ComponentData {
    /// Short name of the kind, for messages.
    pub fn label(&self) -> String {
        match self {
            ComponentData::Camera { .. } => "Camera",
            ComponentData::SkyLight { .. } => "Sky light",
            ComponentData::Light { .. } => "Light",
            ComponentData::Terrain { .. } => "Terrain",
            ComponentData::DebugController => "Debug controller",
            ComponentData::Player => "Player",
            ComponentData::AnimationGraph { .. } => "Animation",
            ComponentData::Model { .. } => "Model",
            ComponentData::Shape { .. } => "Shape",
            ComponentData::Script { name, .. } => return format!("Script {name}"),
        }
        .to_string()
    }

    /// The script `name` with its default data.
    pub fn script(name: &str) -> Self {
        ComponentData::Script {
            name: name.to_string(),
            data: ron::Value::Unit,
        }
    }

    /// Kinds the editor offers in "Add component", with their default data.
    /// Scripts are added by picking one of `script::names()`.
    pub fn catalog() -> Vec<(String, ComponentData)> {
        vec![
            ("Camera".into(), ComponentData::default_camera()),
            ("Sky light".into(), ComponentData::default_sky_light()),
            ("Point light".into(), ComponentData::light(LightSettings::default())),
            (
                "Spot light".into(),
                ComponentData::light(LightSettings {
                    kind: LightKind::Spot,
                    color: [1.0, 0.95, 0.85],
                    intensity: 30.0,
                    range: 25.0,
                    ..LightSettings::default()
                }),
            ),
            (
                "Directional light".into(),
                ComponentData::light(LightSettings {
                    kind: LightKind::Directional,
                    color: [0.6, 0.7, 1.0],
                    intensity: 0.5,
                    bounce: 0.0,
                    ..LightSettings::default()
                }),
            ),
            ("Terrain".into(), ComponentData::bare_terrain(2)),
            ("Debug controller".into(), ComponentData::DebugController),
            ("Shape".into(), ComponentData::default_shape()),
        ]
    }

    pub fn default_camera() -> Self {
        ComponentData::Camera {
            position: [0.0, 60.0, -20.0],
            yaw_deg: 90.0,
            pitch_deg: -20.0,
            fov_deg: 64.0,
            near: 0.1,
            far: 1000.0,
            speed: 10.0,
            free: true,
        }
    }

    pub fn default_shape() -> Self {
        ComponentData::Shape {
            kind: ShapeKind::Cube,
            color: [0.8, 0.8, 0.8],
        }
    }

    /// Terrain without any scattered models; they can be set later.
    pub fn bare_terrain(seed: u64) -> Self {
        ComponentData::Terrain {
            seed,
            scatter: Vec::new(),
        }
    }

    pub fn default_sky_light() -> Self {
        let sky = SkyLight::new((30.0, 20.0, 10.0));
        ComponentData::SkyLight {
            time_of_day: sky.get_time_of_day(),
            day_length_seconds: sky.get_day_length_seconds(),
            paused: sky.is_paused(),
            cloud_coverage: sky.get_cloud_coverage(),
            clouds: *sky.clouds(),
            ambient_occlusion: *sky.ambient_occlusion(),
            hdr: *sky.hdr(),
            weather: *sky.weather(),
            global_illumination: *sky.global_illumination(),
        }
    }

    pub fn light(settings: LightSettings) -> Self {
        let LightSettings {
            kind,
            color,
            intensity,
            range,
            inner_angle,
            outer_angle,
            bounce,
            enabled,
            shadows,
        } = settings;
        ComponentData::Light {
            kind,
            color,
            intensity,
            range,
            inner_angle,
            outer_angle,
            bounce,
            enabled,
            shadows,
        }
    }

    /// The settings of a `Light`, `None` for other kinds.
    pub fn light_settings(&self) -> Option<LightSettings> {
        match *self {
            ComponentData::Light {
                kind,
                color,
                intensity,
                range,
                inner_angle,
                outer_angle,
                bounce,
                enabled,
                shadows,
            } => Some(LightSettings {
                kind,
                color,
                intensity,
                range,
                inner_angle,
                outer_angle,
                bounce,
                enabled,
                shadows,
            }),
            _ => None,
        }
    }

    /// Build the live component and add it to `entity`. `Player` is a prefab
    /// and is handled by `Scene::instantiate`.
    pub fn attach(&self, entity: &mut Entity) {
        match self {
            ComponentData::Camera {
                position,
                yaw_deg,
                pitch_deg,
                fov_deg,
                near,
                far,
                speed,
                free,
            } => {
                let camera = Camera::new(*position, Deg(*yaw_deg), Deg(*pitch_deg));
                // The real aspect is applied every frame by whoever renders.
                let projection = Projection::new(1280, 720, Deg(*fov_deg), *near, *far);
                let mut controller =
                    CameraController::new(*speed, crate::core::camera::DEFAULT_MOUSE_SENSITIVITY);
                controller.is_free = *free;
                entity.add_component(CameraComponent::new(camera, projection, controller));
            }
            ComponentData::SkyLight {
                time_of_day,
                day_length_seconds,
                paused,
                cloud_coverage,
                clouds,
                ambient_occlusion,
                hdr,
                weather,
                global_illumination,
            } => {
                let mut sky = SkyLight::new((30.0, 20.0, 10.0));
                sky.set_time_of_day(*time_of_day);
                sky.set_day_length_seconds(*day_length_seconds);
                sky.set_paused(*paused);
                sky.set_cloud_coverage(*cloud_coverage);
                sky.set_clouds(*clouds);
                sky.set_ambient_occlusion(*ambient_occlusion);
                sky.set_hdr(*hdr);
                sky.set_weather(*weather);
                sky.set_global_illumination(*global_illumination);
                entity.add_component(sky);
            }
            ComponentData::Light { .. } => {
                let settings = self.light_settings().expect("a light");
                entity.add_component(Light::new(settings));
            }
            ComponentData::Terrain { seed, scatter } => {
                entity.add_component(
                    Terrain::<DualContouringChunk>::new(*seed).with_scatter(scatter.clone()),
                );
            }
            ComponentData::DebugController => entity.add_component(DebugController::new()),
            ComponentData::Script { name, data } => match script::create(name, data) {
                Ok(component) => entity.add_boxed_component(component),
                Err(e) => {
                    log::warn!("could not build script '{name}': {e}");
                    entity.add_component(MissingComponent::new(self.clone(), e));
                }
            },
            ComponentData::Player => {}
            ComponentData::AnimationGraph { path } => {
                let result = match entity.get_component_mut::<AnimationComponent>() {
                    Some(animation) => animation.load_graph(path),
                    None => load_graph_file(path)
                        .and_then(|file| build_animation_graph(&file))
                        .map(|graph| entity.add_component(AnimationComponent::with_path(graph, path))),
                };
                if let Err(e) = result {
                    log::warn!("could not load animation graph {path}: {e}");
                    // A prefab's own graph keeps playing; without one the
                    // setting is kept for when the graph can be loaded.
                    if entity.get_component_mut::<AnimationComponent>().is_none() {
                        entity.add_component(MissingComponent::new(self.clone(), e.to_string()));
                    }
                }
            }
            ComponentData::Shape { kind, color } => match ShapeComponent::new(*kind, *color) {
                Ok(shape) => entity.add_component(shape),
                Err(e) => {
                    log::warn!("could not build shape: {e}");
                    entity.add_component(MissingComponent::new(self.clone(), e));
                }
            },
            ComponentData::Model { asset } => {
                let result = match entity.get_component_mut::<ModelComponent>() {
                    // A prefab was already built with this model.
                    Some(model) if model.asset() == Some(asset) => Ok(()),
                    Some(model) => model.set_asset(asset.clone()),
                    None => ModelComponent::from_asset(asset)
                        .map(|model| entity.add_component(model)),
                };
                if let Err(e) = result {
                    log::warn!("could not load model asset {}: {e}", asset.id);
                    if entity.get_component_mut::<ModelComponent>().is_none() {
                        entity.add_component(MissingComponent::new(self.clone(), e));
                    }
                }
            }
        }
    }
}

impl Entity {
    /// Snapshot of this entity and its persistent subtree. Transient
    /// entities (e.g. terrain chunks) return `None`.
    pub fn to_data(&self) -> Option<EntityData> {
        if self.is_transient() {
            return None;
        }
        let p = self.get_position();
        let r = self.get_rotation();
        let s = self.get_scale();
        Some(EntityData {
            id: u64::from(self.id),
            name: self.get_name(),
            position: [p.x, p.y, p.z],
            rotation: [r.s, r.v.x, r.v.y, r.v.z],
            scale: [s.x, s.y, s.z],
            components: self.components().iter().filter_map(|c| c.to_data()).collect(),
            children: self.get_children().iter().filter_map(|c| c.to_data()).collect(),
        })
    }
}

fn unit_scale() -> [f32; 3] {
    [1.0; 3]
}

fn is_unit_scale(scale: &[f32; 3]) -> bool {
    *scale == unit_scale()
}

impl EntityData {
    fn position(&self) -> Point3<f32> {
        Point3::new(self.position[0], self.position[1], self.position[2])
    }

    fn rotation(&self) -> Quaternion<f32> {
        let [w, x, y, z] = self.rotation;
        Quaternion::new(w, x, y, z)
    }

    pub fn is_player(&self) -> bool {
        self.components.contains(&ComponentData::Player)
    }

    /// The model asset the entity is given, if any.
    fn model_asset(&self) -> Option<&AssetRef> {
        self.components.iter().find_map(|component| match component {
            ComponentData::Model { asset } => Some(asset),
            _ => None,
        })
    }
}

impl SceneFile {
    pub fn to_ron(&self) -> Result<String, ron::Error> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
    }

    pub fn from_ron(text: &str) -> Result<Self, ron::error::SpannedError> {
        ron::from_str(text)
    }
}

impl Scene {
    pub fn to_file(&self) -> SceneFile {
        SceneFile {
            version: SCENE_FILE_VERSION,
            settings: SceneSettings {
                shadow_map_size: self.get_shadow_map_size(),
                debug_map_size: self.get_debug_map_size(),
            },
            entities: self
                .get_entities()
                .iter()
                .filter_map(|e| e.to_data())
                .collect(),
        }
    }

    pub fn from_file(file: &SceneFile) -> Result<Scene, Box<dyn Error>> {
        if file.version > SCENE_FILE_VERSION {
            return Err(format!(
                "scene file version {} is newer than supported version {}",
                file.version, SCENE_FILE_VERSION
            )
            .into());
        }
        let mut scene = Scene::new();
        if let Some((w, h)) = file.settings.shadow_map_size {
            scene.add_shadow_map(w, h);
        }
        if let Some((w, h)) = file.settings.debug_map_size {
            scene.add_debug_maps(w, h);
        }
        for data in &file.entities {
            let entity = scene.instantiate(data, false)?;
            scene.add_entity(entity);
        }
        Ok(scene)
    }

    pub fn save_to_path(&self, path: &Path) -> Result<(), Box<dyn Error>> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                fs::create_dir_all(dir)?;
            }
        }
        fs::write(path, self.to_file().to_ron()?)?;
        Ok(())
    }

    /// What could not be built when the scene was loaded, one line per
    /// missing component: `entity: component: reason`.
    pub fn problems(&self) -> Vec<String> {
        fn visit(entity: &Entity, out: &mut Vec<String>) {
            for component in entity.components() {
                if let Some(missing) = component.as_any().downcast_ref::<MissingComponent>() {
                    out.push(format!(
                        "{}: {}: {}",
                        entity.get_name(),
                        missing.data().label(),
                        missing.reason()
                    ));
                }
            }
            for child in entity.get_children() {
                visit(child, out);
            }
        }
        let mut problems = Vec::new();
        for entity in self.get_entities() {
            visit(entity, &mut problems);
        }
        problems
    }

    pub fn load_from_path(path: &Path) -> Result<Scene, Box<dyn Error>> {
        let text = fs::read_to_string(path)?;
        let file = SceneFile::from_ron(&text)?;
        Scene::from_file(&file)
    }

    /// Build a live entity (and subtree) from its description. With
    /// `fresh_ids` every entity gets a new id, as needed when duplicating.
    pub fn instantiate(
        &mut self,
        data: &EntityData,
        fresh_ids: bool,
    ) -> Result<Entity, Box<dyn Error>> {
        let plain = |data: &EntityData| {
            let mut entity = Entity::new(&data.name);
            entity.set_local_position(data.position());
            entity
        };
        let mut entity = if data.is_player() {
            let player = default_animation_graph()
                .and_then(|graph| Player::with_model(self, data.position(), graph, data.model_asset()));
            match player {
                Ok(player) => player,
                // Keep the entity and what it should be, so the rest of the
                // scene loads and saving it loses nothing.
                Err(e) => {
                    let reason = format!(
                        "{e} (the player prefab needs assets/models/{} and the animations \
                         in assets/animations; a new project gets them from the engine's \
                         assets folder)",
                        crate::player::PLAYER_MODEL_FILE
                    );
                    log::warn!("could not build player '{}': {reason}", data.name);
                    let mut entity = plain(data);
                    entity.add_component(MissingComponent::new(ComponentData::Player, reason));
                    entity
                }
            }
        } else {
            plain(data)
        };
        entity.set_name(data.name.clone());
        entity.set_local_rotation(data.rotation());
        entity.set_local_scale(data.scale.into());
        if !fresh_ids {
            entity.id = EntityHandle::from(data.id);
        }
        for component in &data.components {
            component.attach(&mut entity);
        }
        for child in &data.children {
            let child = self.instantiate(child, fresh_ids)?;
            entity.add_child(child);
        }
        Ok(entity)
    }
}

/// The scene a new project starts with (also what `ferrite-mcp` writes for
/// one): a free-flying camera above the terrain, a sky light in the
/// morning and a terrain. Ids are fixed; they only have to be unique
/// within the scene.
pub const STARTER_SCENE: &str = include_str!("starter_scene.ron");

/// The scene the editor and sandbox start with: camera, sky, terrain with a
/// player standing on it, and the debug controller.
pub fn default_scene_file() -> SceneFile {
    fn entity(name: &str, position: [f32; 3], components: Vec<ComponentData>) -> EntityData {
        EntityData {
            id: u64::from(EntityHandle::new()),
            name: name.into(),
            position,
            rotation: [1.0, 0.0, 0.0, 0.0],
            scale: unit_scale(),
            components,
            children: Vec::new(),
        }
    }
    let mut terrain = entity(
        "terrain",
        [0.0; 3],
        vec![ComponentData::Terrain {
            seed: 2,
            scatter: Scatter::default_layers(),
        }],
    );
    terrain
        .children
        .push(entity("player", [0.0, 52.0, 0.0], vec![ComponentData::Player]));
    SceneFile {
        version: SCENE_FILE_VERSION,
        settings: SceneSettings {
            shadow_map_size: Some((4096, 4096)),
            debug_map_size: Some((1280, 720)),
        },
        entities: vec![
            entity(
                "camera",
                [0.0; 3],
                vec![ComponentData::Camera {
                    position: [0.0, 0.0, 0.0],
                    yaw_deg: 0.0,
                    pitch_deg: 0.0,
                    fov_deg: 64.0,
                    near: 0.1,
                    far: 1000.0,
                    speed: 10.0,
                    free: false,
                }],
            ),
            entity("skylight", [0.0; 3], vec![ComponentData::default_sky_light()]),
            terrain,
            entity("debug", [0.0; 3], vec![ComponentData::DebugController]),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::assets::AssetId;
    use crate::core::entity::component::Component;

    #[test]
    fn the_starter_scene_parses_and_has_a_free_camera_sky_and_terrain() {
        let file = SceneFile::from_ron(STARTER_SCENE).unwrap();
        let components: Vec<&ComponentData> =
            file.entities.iter().flat_map(|e| &e.components).collect();
        assert!(components
            .iter()
            .any(|c| matches!(c, ComponentData::Camera { free: true, .. })));
        assert!(components
            .iter()
            .any(|c| matches!(c, ComponentData::SkyLight { time_of_day, .. } if *time_of_day > 0.3)));
        assert!(components.iter().any(|c| matches!(c, ComponentData::Terrain { .. })));
        let mut ids: Vec<u64> = file.entities.iter().map(|e| e.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), file.entities.len());
    }

    #[test]
    fn a_shape_and_a_free_camera_round_trip_through_ron() {
        let shape = ComponentData::Shape {
            kind: ShapeKind::Sphere,
            color: [1.0, 0.5, 0.0],
        };
        let text = ron::to_string(&shape).unwrap();
        assert_eq!(ron::from_str::<ComponentData>(&text).unwrap(), shape);
        // A camera saved before `free` existed is not free.
        let old = "Camera(position:(0.0,1.0,2.0),yaw_deg:0.0,pitch_deg:0.0,fov_deg:60.0,near:0.1,far:100.0,speed:5.0)";
        assert!(matches!(
            ron::from_str::<ComponentData>(old).unwrap(),
            ComponentData::Camera { free: false, .. }
        ));
    }

    #[test]
    fn a_component_that_could_not_be_built_is_saved_as_what_it_should_be() {
        let mut entity = Entity::new("player");
        entity.add_component(MissingComponent::new(
            ComponentData::Player,
            "Unable to open file",
        ));
        let model = ComponentData::Model {
            asset: AssetRef::whole(AssetId::from(7)),
        };
        entity.add_component(MissingComponent::new(model.clone(), "asset does not exist"));

        let data = entity.to_data().unwrap();
        assert_eq!(data.components, vec![ComponentData::Player, model]);
        // It is still a player as far as loading is concerned.
        assert!(data.is_player());
        assert_eq!(data.model_asset(), Some(&AssetRef::whole(AssetId::from(7))));
    }

    #[test]
    fn entity_scale_is_saved_only_when_it_is_not_one() {
        let text = r#"(
            version: 1,
            entities: [(
                id: 1,
                name: "rock",
                position: (0.0, 0.0, 0.0),
                rotation: (1.0, 0.0, 0.0, 0.0),
                scale: (2.0, 1.0, 0.5),
            ), (
                id: 2,
                name: "old",
                position: (0.0, 0.0, 0.0),
                rotation: (1.0, 0.0, 0.0, 0.0),
            )],
        )"#;
        let file = SceneFile::from_ron(text).unwrap();
        assert_eq!(file.entities[0].scale, [2.0, 1.0, 0.5]);
        assert_eq!(file.entities[1].scale, [1.0; 3]);

        let saved = file.to_ron().unwrap();
        assert_eq!(saved.matches("scale").count(), 1);
        let again = SceneFile::from_ron(&saved).unwrap();
        assert_eq!(again.entities[0].scale, [2.0, 1.0, 0.5]);
    }

    #[test]
    fn scenes_from_before_assets_still_parse() {
        let text = r#"(
            version: 1,
            entities: [(
                id: 1,
                name: "terrain",
                position: (0.0, 0.0, 0.0),
                rotation: (1.0, 0.0, 0.0, 0.0),
                components: [Terrain(seed: 2), Player],
            )],
        )"#;
        let file = SceneFile::from_ron(text).unwrap();
        assert_eq!(
            file.entities[0].components,
            vec![ComponentData::bare_terrain(2), ComponentData::Player]
        );
    }

    #[test]
    fn a_light_round_trips_and_may_leave_out_any_field() {
        let data = ComponentData::light(LightSettings {
            kind: LightKind::Spot,
            color: [0.2, 0.5, 1.0],
            intensity: 12.0,
            range: 18.0,
            inner_angle: 15.0,
            outer_angle: 25.0,
            bounce: 0.5,
            enabled: false,
            shadows: false,
        });
        let text = ron::to_string(&data).unwrap();
        assert!(text.starts_with("Light("), "{text}");
        assert_eq!(ron::from_str::<ComponentData>(&text).unwrap(), data);

        let short = ron::from_str::<ComponentData>("Light(kind: Spot, color: (1.0, 0.0, 0.0))")
            .unwrap()
            .light_settings()
            .unwrap();
        assert_eq!(short.kind, LightKind::Spot);
        assert_eq!(short.color, [1.0, 0.0, 0.0]);
        assert_eq!(short.range, LightSettings::default().range);
        assert!(short.enabled);
        assert_eq!(
            ron::from_str::<ComponentData>("Light()").unwrap().light_settings(),
            Some(LightSettings::default())
        );
    }

    #[test]
    fn a_light_attaches_as_a_light_component_and_saves_back() {
        let data = ComponentData::light(LightSettings {
            kind: LightKind::Directional,
            ..LightSettings::default()
        });
        let mut entity = Entity::new("lamp");
        data.attach(&mut entity);
        let light = entity.get_own_component::<Light>().expect("a light");
        assert_eq!(light.settings().kind, LightKind::Directional);
        assert_eq!(light.to_data(), Some(data));
    }

    #[test]
    fn the_catalog_offers_every_kind_of_light() {
        let kinds: Vec<LightKind> = ComponentData::catalog()
            .iter()
            .filter_map(|(_, data)| data.light_settings())
            .map(|s| s.kind)
            .collect();
        assert_eq!(kinds, LightKind::ALL);
    }

    #[test]
    fn sky_lights_from_before_global_illumination_get_its_defaults() {
        let old = "SkyLight(time_of_day: 0.5, day_length_seconds: 60.0, paused: false, cloud_coverage: 0.3)";
        match ron::from_str::<ComponentData>(old).unwrap() {
            ComponentData::SkyLight { global_illumination, .. } => {
                assert_eq!(global_illumination, GlobalIllumination::default())
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_model_component_round_trips() {
        let data = ComponentData::Model {
            asset: AssetRef::part(AssetId::from(0xbeef), "Oak"),
        };
        let text = ron::to_string(&data).unwrap();
        assert_eq!(ron::from_str::<ComponentData>(&text).unwrap(), data);
        assert_eq!(data.label(), "Model");
    }
}
