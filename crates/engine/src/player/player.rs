use cgmath::Point3;
use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};

use crate::core::{
    assets::{self, AssetRef},
    entity::{
        component::{animation_component::AnimationComponent, model_component::ModelComponent},
        Entity,
    },
    model::{animation_graph::AnimationGraph, ModelBuilder},
    physics::rigidbody::RigidBody,
    scene::Scene,
};

use super::{BoneColliders, Player};

const CAPSULE_HALF_HEIGHT: f32 = 0.5;
const CAPSULE_RADIUS: f32 = 0.5;

/// The model the player has unless it is given another one, as a file in
/// `assets/models/`.
pub const PLAYER_MODEL_FILE: &str = "Mannequin.fbx";

/// The asset of the default player model, if that file is registered.
pub fn default_player_model() -> Option<AssetRef> {
    let database = assets::database();
    let source = database.root().join("models").join(PLAYER_MODEL_FILE);
    database.find_by_source(&source).map(AssetRef::whole)
}

/// Load a player model: the given asset, or the default model — as an asset
/// if it is registered, from its file otherwise. Must run on the GL thread.
pub fn load_player_model(
    asset: Option<&AssetRef>,
) -> Result<ModelComponent, Box<dyn std::error::Error>> {
    if let Some(asset) = asset.cloned().or_else(default_player_model) {
        // The asset's scale turns the file's units into metres.
        return Ok(ModelComponent::from_asset(&asset)?);
    }
    let mut model = ModelBuilder::new(PLAYER_MODEL_FILE)?.build();
    model.init();
    Ok(ModelComponent::new(model))
}

impl Player {
    pub fn new<P: Into<Point3<f32>>>(
        scene: &mut Scene,
        position: P,
        animation_graph: AnimationGraph,
    ) -> Result<Entity, Box<dyn std::error::Error>> {
        Self::with_model(scene, position, animation_graph, None)
    }

    /// A player showing `model` instead of the default player model.
    pub fn with_model<P: Into<Point3<f32>>>(
        scene: &mut Scene,
        position: P,
        animation_graph: AnimationGraph,
        model: Option<&AssetRef>,
    ) -> Result<Entity, Box<dyn std::error::Error>> {
        // Before anything is added to the scene: this is the step that
        // fails when assets are missing.
        let model_component = load_player_model(model)?;

        let mut entity = Entity::new("player");
        entity.set_position(scene, position);

        // Player::new is given the default player graph; a scene's own graph
        // file replaces it via ComponentData::AnimationGraph.
        let animation_component =
            AnimationComponent::with_path(animation_graph, super::PLAYER_GRAPH_PATH);

        let collider = ColliderBuilder::capsule_y(CAPSULE_HALF_HEIGHT, CAPSULE_RADIUS)
            .friction(0.0)
            .build();

        let translation = entity.get_position();
        let rigid_body = RigidBodyBuilder::dynamic()
            .translation(rapier3d::prelude::Vector::new(
                translation.x,
                translation.y,
                translation.z,
            ))
            .lock_rotations()
            .build();
        let rigid_body_handle = scene.physics_engine.add_rigid_body(rigid_body);
        scene
            .physics_engine
            .add_collider(collider, Some(rigid_body_handle));

        let rigidbody_component = RigidBody {
            rigid_body_handle,
            show_collider: false,
        };
        entity.add_component(animation_component);
        entity.add_component(rigidbody_component);
        entity.add_component(model_component);

        let bone_colliders = BoneColliders::build(scene, &entity);
        entity.add_component(bone_colliders);

        Ok(entity)
    }
}
