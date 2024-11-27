mod player;

pub struct Player {}

pub struct PlayerController {
    forward: f32,
    backward: f32,
    left: f32,
    right: f32,

    dirty: bool,
}
