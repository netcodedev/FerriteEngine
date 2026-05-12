mod bone_colliders;
mod player;

pub use bone_colliders::BoneColliders;
use rapier3d::prelude::RigidBodyHandle;

pub struct Player {}

pub struct PlayerController {
    forward: f32,
    backward: f32,
    left: f32,
    right: f32,
    dirty: bool,
    rigid_body_handle: RigidBodyHandle,
}
