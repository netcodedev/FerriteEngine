//! Serializable scene description (RON on disk) and the registry that turns
//! it back into live entities and components.

use std::{error::Error, fs, path::Path};

use cgmath::{Deg, Point3, Quaternion};
use serde::{Deserialize, Serialize};

use crate::{
    core::{
        camera::{Camera, CameraController, Projection},
        entity::{
            component::{camera_component::CameraComponent, debug_component::DebugController},
            Entity, EntityHandle,
        },
        renderer::light::skylight::SkyLight,
    },
    player::{default_animation_graph, Player},
    terrain::{dual_contouring::DualContouringChunk, Terrain},
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
    },
    SkyLight {
        time_of_day: f32,
        day_length_seconds: f32,
        paused: bool,
        cloud_coverage: f32,
    },
    Terrain {
        seed: u64,
    },
    DebugController,
    /// Prefab marker: the entity is rebuilt with `Player::new`, which brings
    /// its model, animation graph, physics capsule and bone colliders.
    Player,
}

impl ComponentData {
    /// Kinds the editor offers in "Add component", with their default data.
    pub fn catalog() -> Vec<(&'static str, ComponentData)> {
        vec![
            ("Camera", ComponentData::default_camera()),
            ("Sky light", ComponentData::default_sky_light()),
            ("Terrain", ComponentData::Terrain { seed: 2 }),
            ("Debug controller", ComponentData::DebugController),
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
        }
    }

    pub fn default_sky_light() -> Self {
        let sky = SkyLight::new((30.0, 20.0, 10.0));
        ComponentData::SkyLight {
            time_of_day: sky.get_time_of_day(),
            day_length_seconds: sky.get_day_length_seconds(),
            paused: sky.is_paused(),
            cloud_coverage: sky.get_cloud_coverage(),
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
            } => {
                let camera = Camera::new(*position, Deg(*yaw_deg), Deg(*pitch_deg));
                // The real aspect is applied every frame by whoever renders.
                let projection = Projection::new(1280, 720, Deg(*fov_deg), *near, *far);
                let controller = CameraController::new(*speed, 1.0);
                entity.add_component(CameraComponent::new(camera, projection, controller));
            }
            ComponentData::SkyLight {
                time_of_day,
                day_length_seconds,
                paused,
                cloud_coverage,
            } => {
                let mut sky = SkyLight::new((30.0, 20.0, 10.0));
                sky.set_time_of_day(*time_of_day);
                sky.set_day_length_seconds(*day_length_seconds);
                sky.set_paused(*paused);
                sky.set_cloud_coverage(*cloud_coverage);
                entity.add_component(sky);
            }
            ComponentData::Terrain { seed } => {
                entity.add_component(Terrain::<DualContouringChunk>::new(*seed));
            }
            ComponentData::DebugController => entity.add_component(DebugController::new()),
            ComponentData::Player => {}
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
        Some(EntityData {
            id: u64::from(self.id),
            name: self.get_name(),
            position: [p.x, p.y, p.z],
            rotation: [r.s, r.v.x, r.v.y, r.v.z],
            components: self.components().iter().filter_map(|c| c.to_data()).collect(),
            children: self.get_children().iter().filter_map(|c| c.to_data()).collect(),
        })
    }
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
        let mut entity = if data.is_player() {
            Player::new(self, data.position(), default_animation_graph()?)?
        } else {
            let mut entity = Entity::new(&data.name);
            entity.set_local_position(data.position());
            entity
        };
        entity.set_name(data.name.clone());
        entity.set_local_rotation(data.rotation());
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

/// The scene the editor and sandbox start with: camera, sky, terrain with a
/// player standing on it, and the debug controller.
pub fn default_scene_file() -> SceneFile {
    fn entity(name: &str, position: [f32; 3], components: Vec<ComponentData>) -> EntityData {
        EntityData {
            id: u64::from(EntityHandle::new()),
            name: name.into(),
            position,
            rotation: [1.0, 0.0, 0.0, 0.0],
            components,
            children: Vec::new(),
        }
    }
    let mut terrain = entity("terrain", [0.0; 3], vec![ComponentData::Terrain { seed: 2 }]);
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
                }],
            ),
            entity("skylight", [0.0; 3], vec![ComponentData::default_sky_light()]),
            terrain,
            entity("debug", [0.0; 3], vec![ComponentData::DebugController]),
        ],
    }
}
