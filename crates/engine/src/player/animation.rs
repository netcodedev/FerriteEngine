use std::error::Error;

use crate::core::model::{
    animation_graph::{AnimationGraph, State},
    Animation,
};

/// Locomotion graph for the mannequin player (idle, walk, strafe, run, jump).
/// Lives in the engine so scenes loaded from disk can rebuild the player.
pub fn default_animation_graph() -> Result<AnimationGraph, Box<dyn Error>> {
    // Animation state graph:
    //
    //   forward_left ─── walk ──[sprint]──► fast_run ─── forward_right
    //        |             |                   |                |
    //       left ────────  idle  ───────────────────────────  right
    //        |             |                                    |
    //   backward_left ─── back ──────────────────────── backward_right
    //
    //   Any locomotion state ──[jump]──► jump ──[released]──► idle

    let mut animation_graph = AnimationGraph::new();
    animation_graph.add_input("forward", 0.0);
    animation_graph.add_input("backward", 0.0);
    animation_graph.add_input("left", 0.0);
    animation_graph.add_input("right", 0.0);
    animation_graph.add_input("sprint", 0.0);
    animation_graph.add_input("jump", 0.0);

    let transition_speed = 0.5;

    let mut idle_state = State::new("idle");
    idle_state.add_animation(Animation::from_file("idle", "Idle.fbx")?);
    idle_state.add_transition(
        "walk",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] > 0.0 && inputs["left"] - inputs["right"] == 0.0
        }),
        transition_speed,
    );
    idle_state.add_transition(
        "back",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] < 0.0 && inputs["left"] - inputs["right"] == 0.0
        }),
        transition_speed,
    );
    idle_state.add_transition(
        "left",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] == 0.0 && inputs["left"] - inputs["right"] > 0.0
        }),
        transition_speed,
    );
    idle_state.add_transition(
        "right",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] == 0.0 && inputs["left"] - inputs["right"] < 0.0
        }),
        transition_speed,
    );
    idle_state.add_transition(
        "jump",
        Box::new(|inputs| inputs["jump"] > 0.0),
        transition_speed,
    );
    animation_graph.set_default_state(idle_state);

    let mut walk_state = State::new("walk");
    walk_state.add_animation(Animation::from_file("walk", "Walk.fbx")?);
    walk_state.add_transition(
        "idle",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] == 0.0 && inputs["left"] - inputs["right"] == 0.0
        }),
        transition_speed,
    );
    walk_state.add_transition(
        "forward_left",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] > 0.0 && inputs["left"] - inputs["right"] > 0.0
        }),
        transition_speed,
    );
    walk_state.add_transition(
        "forward_right",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] > 0.0 && inputs["left"] - inputs["right"] < 0.0
        }),
        transition_speed,
    );
    walk_state.add_transition(
        "fast_run",
        Box::new(|inputs| {
            inputs["sprint"] > 0.0
                && inputs["forward"] - inputs["backward"] > 0.0
                && inputs["left"] - inputs["right"] == 0.0
        }),
        transition_speed,
    );
    walk_state.add_transition(
        "jump",
        Box::new(|inputs| inputs["jump"] > 0.0),
        transition_speed,
    );
    animation_graph.add_state(walk_state);

    let mut fast_run_state = State::new("fast_run");
    fast_run_state.add_animation(Animation::from_file("fast_run", "Fast Run.fbx")?);
    fast_run_state.add_transition(
        "idle",
        Box::new(|inputs| inputs["forward"] - inputs["backward"] <= 0.0),
        transition_speed,
    );
    fast_run_state.add_transition(
        "walk",
        Box::new(|inputs| {
            inputs["sprint"] == 0.0 && inputs["forward"] - inputs["backward"] > 0.0
        }),
        transition_speed,
    );
    fast_run_state.add_transition(
        "jump",
        Box::new(|inputs| inputs["jump"] > 0.0),
        transition_speed,
    );
    animation_graph.add_state(fast_run_state);

    let mut back_state = State::new("back");
    back_state.add_animation(Animation::from_file("back", "Walk_Backwards.fbx")?);
    back_state.add_transition(
        "idle",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] == 0.0 && inputs["left"] - inputs["right"] == 0.0
        }),
        transition_speed,
    );
    back_state.add_transition(
        "backward_left",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] < 0.0 && inputs["left"] - inputs["right"] > 0.0
        }),
        transition_speed,
    );
    back_state.add_transition(
        "backward_right",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] < 0.0 && inputs["left"] - inputs["right"] < 0.0
        }),
        transition_speed,
    );
    back_state.add_transition(
        "jump",
        Box::new(|inputs| inputs["jump"] > 0.0),
        transition_speed,
    );
    animation_graph.add_state(back_state);

    let mut left_state = State::new("left");
    left_state.add_animation(Animation::from_file("left", "Walk_Left.fbx")?);
    left_state.add_transition(
        "idle",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] == 0.0 && inputs["left"] - inputs["right"] == 0.0
        }),
        transition_speed,
    );
    left_state.add_transition(
        "forward_left",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] > 0.0 && inputs["left"] - inputs["right"] > 0.0
        }),
        transition_speed,
    );
    left_state.add_transition(
        "backward_left",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] < 0.0 && inputs["left"] - inputs["right"] > 0.0
        }),
        transition_speed,
    );
    left_state.add_transition(
        "jump",
        Box::new(|inputs| inputs["jump"] > 0.0),
        transition_speed,
    );
    animation_graph.add_state(left_state);

    let mut right_state = State::new("right");
    right_state.add_animation(Animation::from_file("right", "Walk_Right.fbx")?);
    right_state.add_transition(
        "idle",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] == 0.0 && inputs["left"] - inputs["right"] == 0.0
        }),
        transition_speed,
    );
    right_state.add_transition(
        "forward_right",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] > 0.0 && inputs["left"] - inputs["right"] < 0.0
        }),
        transition_speed,
    );
    right_state.add_transition(
        "backward_right",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] < 0.0 && inputs["left"] - inputs["right"] < 0.0
        }),
        transition_speed,
    );
    right_state.add_transition(
        "jump",
        Box::new(|inputs| inputs["jump"] > 0.0),
        transition_speed,
    );
    animation_graph.add_state(right_state);

    let mut forward_left_state = State::new("forward_left");
    forward_left_state.add_animation(Animation::from_file("walk", "Walk.fbx")?);
    forward_left_state.add_animation(Animation::from_file("left", "Walk_Left.fbx")?);
    forward_left_state.add_transition(
        "walk",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] > 0.0 && inputs["left"] - inputs["right"] == 0.0
        }),
        transition_speed,
    );
    forward_left_state.add_transition(
        "left",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] == 0.0 && inputs["left"] - inputs["right"] > 0.0
        }),
        transition_speed,
    );
    forward_left_state.add_transition(
        "jump",
        Box::new(|inputs| inputs["jump"] > 0.0),
        transition_speed,
    );
    forward_left_state.sync_animations(true);
    animation_graph.add_state(forward_left_state);

    let mut forward_right_state = State::new("forward_right");
    forward_right_state.add_animation(Animation::from_file("walk", "Walk.fbx")?);
    forward_right_state.add_animation(Animation::from_file("right", "Walk_Right.fbx")?);
    forward_right_state.add_transition(
        "walk",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] > 0.0 && inputs["left"] - inputs["right"] == 0.0
        }),
        transition_speed,
    );
    forward_right_state.add_transition(
        "right",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] == 0.0 && inputs["left"] - inputs["right"] < 0.0
        }),
        transition_speed,
    );
    forward_right_state.add_transition(
        "jump",
        Box::new(|inputs| inputs["jump"] > 0.0),
        transition_speed,
    );
    forward_right_state.sync_animations(true);
    animation_graph.add_state(forward_right_state);

    let mut backward_left_state = State::new("backward_left");
    backward_left_state.add_animation(Animation::from_file("back", "Walk_Backwards.fbx")?);
    backward_left_state.add_animation(Animation::from_file("left", "Walk_Left.fbx")?);
    backward_left_state.add_transition(
        "back",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] < 0.0 && inputs["left"] - inputs["right"] == 0.0
        }),
        transition_speed,
    );
    backward_left_state.add_transition(
        "left",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] == 0.0 && inputs["left"] - inputs["right"] > 0.0
        }),
        transition_speed,
    );
    backward_left_state.add_transition(
        "jump",
        Box::new(|inputs| inputs["jump"] > 0.0),
        transition_speed,
    );
    backward_left_state.sync_animations(true);
    animation_graph.add_state(backward_left_state);

    let mut backward_right_state = State::new("backward_right");
    backward_right_state.add_animation(Animation::from_file("back", "Walk_Backwards.fbx")?);
    backward_right_state.add_animation(Animation::from_file("right", "Walk_Right.fbx")?);
    backward_right_state.add_transition(
        "back",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] < 0.0 && inputs["left"] - inputs["right"] == 0.0
        }),
        transition_speed,
    );
    backward_right_state.add_transition(
        "right",
        Box::new(|inputs| {
            inputs["forward"] - inputs["backward"] == 0.0 && inputs["left"] - inputs["right"] < 0.0
        }),
        transition_speed,
    );
    backward_right_state.add_transition(
        "jump",
        Box::new(|inputs| inputs["jump"] > 0.0),
        transition_speed,
    );
    backward_right_state.sync_animations(true);
    animation_graph.add_state(backward_right_state);

    let mut jump_state = State::new("jump");
    jump_state.add_animation(Animation::from_file("jump", "Jump.fbx")?);
    jump_state.add_transition(
        "idle",
        Box::new(|inputs| inputs["jump"] == 0.0),
        transition_speed,
    );
    animation_graph.add_state(jump_state);

    Ok(animation_graph)
}
