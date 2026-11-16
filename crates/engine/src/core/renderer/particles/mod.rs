//! Particles: fire, smoke, embers, sparks, dust, wood chips, fireflies.
//!
//! A `ParticleEmitter` on an entity sends out particles as its
//! `ParticleEffect` says: from a shape around the entity, in a direction,
//! pulled by gravity and wind, slowed by drag, stirred by turbulence,
//! changing colour and size over their life. They are simulated on the CPU
//! (in play mode and in the editor, so effects can be tuned live) and drawn
//! as camera-facing quads in the transparent pass (`render`).
//!
//! An effect either glows (`emissive`, added on top of what is behind: fire,
//! sparks) or is lit like a surface (`lit`: smoke, dust), by the sun, the
//! moon, the sky and the scene's lights. Particles fade where they meet
//! geometry instead of cutting into it (`soft`).
//!
//! One-shot effects (a puff of dust when something is built, chips when a
//! tree is hit) are `spawn_burst`: an entity that removes itself when its
//! particles are gone.

mod effect;
mod emitter;
mod preset;
mod render;
mod system;
#[cfg(test)]
mod tests;

pub use effect::*;
pub use emitter::*;
pub use preset::*;
pub use render::{release_gl_resources, ParticleRenderer};
pub use system::*;
