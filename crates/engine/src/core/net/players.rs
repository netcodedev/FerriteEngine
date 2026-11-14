//! The players of the other peers: what is sent about a player, and the
//! copies (`RemotePlayer`) that show them.
//!
//! A player is an entity of the player prefab (`ComponentData::Player`).
//! The copy of another peer's player is built from the same scene data as
//! this peer's own player, without its scripts: the same model and
//! animation graph, its body moved to where the other player is (a
//! kinematic body, so this player bumps into it), its animation graph fed
//! the other player's inputs.

use std::{cell::RefCell, collections::HashMap};

use cgmath::{InnerSpace, Point3, Quaternion};
use glfw::{Glfw, Window, WindowEvent};
use rapier3d::prelude::RigidBodyType;
use serde::{Deserialize, Serialize};

use crate::{
    core::{
        entity::{
            component::{
                animation_component::AnimationComponent, model_component::ModelComponent,
                Component,
            },
            Entity, EntityHandle,
        },
        physics::rigidbody::RigidBody,
        scene::{
            scene_file::{ComponentData, EntityData},
            Scene,
        },
    },
    player::BoneColliders,
};

use super::ClientId;

/// How quickly a copy catches up with where its player was last seen, per
/// second: the updates arrive 20 times a second and are smoothed over.
const FOLLOW_RATE: f32 = 14.0;
/// Further than this behind, a copy jumps instead (respawn, teleport).
const SNAP_DISTANCE: f32 = 8.0;

/// What is sent about a player.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlayerState {
    pub position: [f32; 3],
    /// Quaternion, w first.
    pub rotation: [f32; 4],
    /// The inputs of its animation graph.
    pub inputs: Vec<(String, f32)>,
}

/// The copy of another peer's player.
pub struct RemotePlayer {
    pub id: ClientId,
    target: PlayerState,
}

impl RemotePlayer {
    fn target_position(&self) -> Point3<f32> {
        self.target.position.into()
    }

    fn target_rotation(&self) -> Quaternion<f32> {
        let [w, x, y, z] = self.target.rotation;
        Quaternion::new(w, x, y, z)
    }
}

impl Component for RemotePlayer {
    fn update(&mut self, scene: &mut Scene, entity: &mut Entity, delta_time: f64) {
        let dt = delta_time as f32;
        let target = self.target_position();
        let now = entity.get_position();
        let position = if (target - now).magnitude() > SNAP_DISTANCE {
            target
        } else {
            let t = 1.0 - (-FOLLOW_RATE * dt).exp();
            now + (target - now) * t
        };
        let t = 1.0 - (-FOLLOW_RATE * dt).exp();
        let rotation = entity.get_rotation().nlerp(self.target_rotation(), t);
        entity.set_position(scene, position);
        entity.set_rotation(scene, rotation);
        if let Some(animation) = entity.get_component_mut::<AnimationComponent>() {
            for (name, value) in &self.target.inputs {
                animation.set_input(name, *value);
            }
        }
        // The body moves as it is told; whatever the animations would move
        // it by is thrown away, as the player's own controller does.
        if let Some(model) = entity.get_component_mut::<ModelComponent>() {
            model.get_model_mut().reset_position();
        }
    }

    fn handle_event(&mut self, _: &mut Glfw, _: &mut Window, _: &WindowEvent) {}

    fn display_name(&self) -> String {
        format!("Remote player {}", self.id)
    }
}

thread_local! {
    /// What a player looks like in this scene (its prefab data without
    /// scripts), taken from the scene's own player while there is one.
    static TEMPLATE: RefCell<Option<EntityData>> = const { RefCell::new(None) };
}

/// The entity itself, not one below it (`get_component` looks into
/// children too).
fn is_local_player(entity: &Entity) -> bool {
    entity.get_own_component::<BoneColliders>().is_some()
        && entity.get_own_component::<RemotePlayer>().is_none()
}

/// Depth first: the first entity (with what is above it) `test` accepts.
fn find<'a>(entities: &'a [Entity], test: &dyn Fn(&Entity) -> bool) -> Option<&'a Entity> {
    entities.iter().find_map(|entity| {
        if test(entity) {
            Some(entity)
        } else {
            find(entity.get_children(), test)
        }
    })
}

fn collect(entities: &[Entity], test: &dyn Fn(&Entity) -> bool, out: &mut Vec<EntityHandle>) {
    for entity in entities {
        if test(entity) {
            out.push(entity.id);
        }
        collect(entity.get_children(), test, out);
    }
}

/// The player prefab's data, without scripts or children.
fn template_of(entity: &Entity) -> Option<EntityData> {
    let mut data = entity.to_data()?;
    data.components
        .retain(|c| !matches!(c, ComponentData::Script { .. }));
    data.children.clear();
    Some(data)
}

/// This peer's own player, as it is to be sent. Remembers what it looks
/// like for the copies.
pub(super) fn local_state(scene: &Scene) -> Option<PlayerState> {
    let player = find(scene.get_entities(), &is_local_player)?;
    if TEMPLATE.with(|t| t.borrow().is_none()) {
        let template = template_of(player);
        TEMPLATE.with(|t| *t.borrow_mut() = template);
    }
    let world = scene.get_world_transform(&player.id)?;
    let position = [world.w.x, world.w.y, world.w.z];
    let r = player.get_rotation();
    let inputs = player
        .get_component::<AnimationComponent>()
        .map(|animation| {
            let mut inputs: Vec<(String, f32)> = animation
                .graph()
                .inputs()
                .iter()
                .map(|(name, value)| (name.clone(), *value))
                .collect();
            inputs.sort_by(|a, b| a.0.cmp(&b.0));
            inputs
        })
        .unwrap_or_default();
    Some(PlayerState {
        position,
        rotation: [r.s, r.v.x, r.v.y, r.v.z],
        inputs,
    })
}

/// Players who join stand this far from the scene's spawn point, each in
/// its own direction, so they do not all stand on one spot.
const SPAWN_SPREAD: f32 = 1.5;

/// Put this peer's own player where it was when it last left the server
/// (`returning`), or apart from the others who joined (`index`: how many
/// joined before).
pub(super) fn place_local(scene: &mut Scene, returning: Option<&PlayerState>, index: u32) {
    let Some(player) = find(scene.get_entities(), &is_local_player) else {
        return;
    };
    let id = player.id;
    let (position, rotation) = match returning {
        Some(state) => {
            let [w, x, y, z] = state.rotation;
            (Point3::from(state.position), Quaternion::new(w, x, y, z))
        }
        None if index == 0 => return,
        None => {
            // Around the spawn point, by the golden angle.
            let angle = index as f32 * 2.399_963;
            let at = player.get_position()
                + cgmath::Vector3::new(angle.cos(), 0.0, angle.sin()) * SPAWN_SPREAD;
            (at, player.get_rotation())
        }
    };
    let handle = player.get_component::<RigidBody>().map(|b| b.rigid_body_handle);
    // The body is where the player is; the entity follows it each update.
    if let Some(handle) = handle {
        let body = &mut scene.physics_engine.rigid_bodies[handle];
        body.set_translation(
            rapier3d::prelude::Vector::new(position.x, position.y, position.z),
            true,
        );
        body.set_rotation(
            rapier3d::prelude::Rotation::from_xyzw(rotation.v.x, rotation.v.y, rotation.v.z, rotation.s),
            true,
        );
        body.set_linvel(rapier3d::prelude::Vector::new(0.0, 0.0, 0.0), true);
    }
    if let Some(entity) = scene.get_entity_mut(&id) {
        entity.set_local_position(position);
        entity.set_local_rotation(rotation);
    }
}

/// Take the scene's own players out (a dedicated server has none),
/// remembering what one looks like.
pub(super) fn remove_local_players(scene: &mut Scene) {
    if let Some(player) = find(scene.get_entities(), &is_local_player) {
        let template = template_of(player);
        TEMPLATE.with(|t| *t.borrow_mut() = template);
    }
    let mut ids = Vec::new();
    collect(scene.get_entities(), &is_local_player, &mut ids);
    for id in ids {
        scene.remove_entity(&id);
    }
}

fn remote_ids(scene: &Scene) -> HashMap<ClientId, EntityHandle> {
    let mut found = HashMap::new();
    fn visit(entities: &[Entity], found: &mut HashMap<ClientId, EntityHandle>) {
        for entity in entities {
            if let Some(remote) = entity.get_own_component::<RemotePlayer>() {
                found.insert(remote.id, entity.id);
            }
            visit(entity.get_children(), found);
        }
    }
    visit(scene.get_entities(), &mut found);
    found
}

/// A copy of player `id` where `state` says.
fn spawn(scene: &mut Scene, id: ClientId, state: &PlayerState) -> Option<()> {
    let mut data = TEMPLATE.with(|t| t.borrow().clone())?;
    data.name = format!("player {id}");
    data.position = state.position;
    data.rotation = state.rotation;
    let mut entity = match scene.instantiate(&data, true) {
        Ok(entity) => entity,
        Err(e) => {
            log::warn!("net: cannot show player {id}: {e}");
            return None;
        }
    };
    // Moved where it is told, pushing what it moves into.
    if let Some(body) = entity.get_component::<RigidBody>() {
        scene.physics_engine.rigid_bodies[body.rigid_body_handle]
            .set_body_type(RigidBodyType::KinematicPositionBased, true);
    }
    entity.set_transient(true);
    entity.add_component(RemotePlayer {
        id,
        target: state.clone(),
    });
    scene.add_entity(entity);
    Some(())
}

/// Copies for the players in `others`, where they are; the ones that `left`
/// leave.
pub(super) fn sync(
    scene: &mut Scene,
    others: &HashMap<ClientId, PlayerState>,
    left: &[ClientId],
    _delta_time: f32,
) {
    let existing = remote_ids(scene);
    for id in left {
        if let Some(entity) = existing.get(id) {
            scene.remove_entity(entity);
        }
    }
    for (id, state) in others {
        match existing.get(id) {
            Some(entity) => {
                if let Some(remote) = scene
                    .get_entity_mut(entity)
                    .and_then(|e| e.get_component_mut::<RemotePlayer>())
                {
                    remote.target = state.clone();
                }
            }
            None => {
                spawn(scene, *id, state);
            }
        }
    }
    // Copies of players no longer listed (the list is complete each time).
    for (id, entity) in existing {
        if !others.contains_key(&id) && !left.contains(&id) {
            scene.remove_entity(&entity);
        }
    }
}

/// No copies offline.
pub(super) fn remove_all(scene: &mut Scene) {
    let existing = remote_ids(scene);
    for entity in existing.values() {
        scene.remove_entity(entity);
    }
}
