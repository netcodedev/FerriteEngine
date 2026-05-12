mod bone_colliders;
mod player;

pub use bone_colliders::BoneColliders;
use cgmath::{Rad, Vector3};
use rapier3d::prelude::RigidBodyHandle;

pub struct Player {}

pub struct PlayerController {
    forward: f32,
    backward: f32,
    left: f32,
    right: f32,
    sprint: f32,
    jump: f32,
    jump_impulse_pending: bool,
    dirty: bool,
    rigid_body_handle: RigidBodyHandle,
    /// Player facing yaw (radians). Camera look direction = (cos yaw, 0, sin yaw).
    yaw: Rad<f32>,
    /// Camera pitch (radians, clamped to ±FRAC_PI_2).
    pitch: Rad<f32>,
    /// Raw mouse delta accumulated this frame (reset after update).
    mouse_dx: f32,
    mouse_dy: f32,
    /// Mouse sensitivity in radians-per-pixel (no dt factor).
    sensitivity: f32,
    /// When true the cursor is visible and the camera moves freely with arrow keys.
    is_free_camera: bool,
    /// Local-space camera offset behind/above the player (rotated by yaw each frame).
    camera_offset: Vector3<f32>,
}
