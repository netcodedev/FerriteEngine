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
            },
            Entity, EntityHandle,
        },
        renderer::light::skylight::SkyLight,
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
        /// Models scattered over the terrain, each with its own density.
        #[serde(default)]
        scatter: Vec<ScatterLayerData>,
    },
    DebugController,
    /// Prefab marker: the entity is rebuilt with `Player::new`, which brings
    /// its model, animation graph, physics capsule and bone colliders.
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
}

impl ComponentData {
    /// Short name of the kind, for messages.
    pub fn label(&self) -> &'static str {
        match self {
            ComponentData::Camera { .. } => "Camera",
            ComponentData::SkyLight { .. } => "Sky light",
            ComponentData::Terrain { .. } => "Terrain",
            ComponentData::DebugController => "Debug controller",
            ComponentData::Player => "Player",
            ComponentData::AnimationGraph { .. } => "Animation",
            ComponentData::Model { .. } => "Model",
        }
    }

    /// Kinds the editor offers in "Add component", with their default data.
    pub fn catalog() -> Vec<(&'static str, ComponentData)> {
        vec![
            ("Camera", ComponentData::default_camera()),
            ("Sky light", ComponentData::default_sky_light()),
            ("Terrain", ComponentData::bare_terrain(2)),
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
            ComponentData::Terrain { seed, scatter } => {
                entity.add_component(
                    Terrain::<DualContouringChunk>::new(*seed).with_scatter(scatter.clone()),
                );
            }
            ComponentData::DebugController => entity.add_component(DebugController::new()),
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
                    log::warn!("could not build player '{}': {e}", data.name);
                    let mut entity = plain(data);
                    entity.add_component(MissingComponent::new(
                        ComponentData::Player,
                        e.to_string(),
                    ));
                    entity
                }
            }
        } else {
            plain(data)
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
    fn a_model_component_round_trips() {
        let data = ComponentData::Model {
            asset: AssetRef::part(AssetId::from(0xbeef), "Oak"),
        };
        let text = ron::to_string(&data).unwrap();
        assert_eq!(ron::from_str::<ComponentData>(&text).unwrap(), data);
        assert_eq!(data.label(), "Model");
    }
}
