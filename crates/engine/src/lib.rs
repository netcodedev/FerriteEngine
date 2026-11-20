pub mod core;
pub mod player;
pub mod terrain;
pub mod vegetation;

// Re-exported for game crates, which use the engine's own copies: a game
// library links the engine dynamically and must not bring second copies.
pub use cgmath;
pub use image;
pub use glfw;
pub use log;
pub use rapier3d;
pub use russimp_ng;
pub use ron;
pub use serde;
pub use bincode;

// Lets `#[derive(Script)]` refer to `::ferrite` inside the engine as well.
extern crate self as ferrite;

pub use ferrite_macros::Script;

/// What a script file usually needs: `use ferrite::prelude::*;`.
pub mod prelude {
    pub use crate::core::{
        entity::{
            component::{Component, Property, PropertyValue},
            Entity,
        },
        scene::Scene,
        script::{Behaviour, Script},
    };
    pub use crate::Script;
    pub use cgmath::{
        Deg, EuclideanSpace, InnerSpace, Matrix4, Point3, Quaternion, Rad, Rotation3, Vector3,
        Zero,
    };
    pub use glfw::{Action, Key, MouseButton, WindowEvent};
    // `info!("...")` and friends: shown in the editor's console.
    pub use log::{debug, error, info, trace, warn};
    pub use serde::{Deserialize, Serialize};
}
