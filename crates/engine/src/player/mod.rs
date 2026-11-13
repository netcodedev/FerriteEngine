mod animation;
mod bone_colliders;
mod player;

pub use animation::{
    animation_graph_files, build_animation_graph, default_animation_graph, load_graph_file,
    player_graph_file, GRAPH_DIR, GRAPH_EXTENSION, PLAYER_GRAPH_PATH,
};

pub use bone_colliders::BoneColliders;
pub use player::{default_player_model, load_player_model, PLAYER_MODEL_FILE};
/// Builds the player's body: model, animation, physics capsule and bone
/// colliders. What moves it is up to the game.
pub struct Player {}
