//! Something a character holds: a model drawn in its hand, following the
//! hand through every animation, held as the grip on the model's asset
//! says (`ModelSettings::grip`, set in the editor's asset workspace).
//! Held with both hands, the other hand is reached to the grip's second
//! point by inverse kinematics.
//!
//! Goes on the entity with the character's `ModelComponent`, after its
//! `AnimationComponent` (it poses the other hand after the animation).
//! How much the other hand holds on is up to the game
//! (`set_other_hand_weight`): an axe carried at the side is held in one
//! hand and gripped with both for a swing. Out of reach of the other
//! hand, it lets go.

use cgmath::Matrix4;
use glfw::{Glfw, WindowEvent};

use crate::core::{
    assets::{self, AssetRef},
    entity::Entity,
    model::grip::{self, Grip, HeldPlacement},
    scene::Scene,
};

pub use crate::core::model::grip::Hand;

use super::{model_component::ModelComponent, shared_model_component::SharedModel, Component};

pub struct HeldItem {
    model: SharedModel,
    /// The grip of the model's asset, as last read.
    grip: Grip,
    hand: Hand,
    /// How much the other hand holds the second grip: 0 not, 1 fully.
    other_hand_weight: f32,
    /// Where it is held, worked out once for the passes of a frame.
    placement: HeldPlacement,
}

/// The grip set on `asset`, or one at the model's origin.
fn grip_of(asset: &AssetRef) -> Grip {
    assets::database()
        .get(asset.id)
        .and_then(|entry| entry.meta.as_model()?.grip.clone())
        .unwrap_or_else(|| Grip::at([0.0; 3]))
}

impl HeldItem {
    /// Hold the model of `asset` in `hand`. Must run on the GL thread.
    pub fn new(asset: &AssetRef, hand: Hand) -> Result<Self, String> {
        Ok(Self {
            model: SharedModel::from_asset(asset)?,
            grip: grip_of(asset),
            hand,
            other_hand_weight: 1.0,
            placement: HeldPlacement::default(),
        })
    }

    pub fn asset(&self) -> &AssetRef {
        self.model.asset()
    }

    pub fn grip(&self) -> &Grip {
        &self.grip
    }

    /// How much the other hand holds the grip's second point (if it has
    /// one), from 0 (it does what the animation says) to 1 (the default).
    pub fn set_other_hand_weight(&mut self, weight: f32) {
        self.other_hand_weight = weight.clamp(0.0, 1.0);
    }
}

impl Component for HeldItem {
    fn update(&mut self, _: &mut Scene, entity: &mut Entity, _: f64) {
        // A grip changed in the editor shows at once.
        self.grip = grip_of(self.model.asset());
        if self.grip.other_hand.is_none() {
            return;
        }
        let transform = entity.local_transform();
        if let Some(model) = entity.get_component_mut::<ModelComponent>() {
            grip::reach_with_other_hand(
                model.get_model_mut(),
                &transform,
                &self.grip,
                self.hand,
                self.other_hand_weight,
            );
        }
    }

    fn render(
        &self,
        scene: &Scene,
        entity: &Entity,
        view_projection: &Matrix4<f32>,
        parent_transform: &Matrix4<f32>,
    ) {
        // The ground map holds the terrain alone.
        if crate::core::renderer::weather::exposure::in_ground_pass() {
            return;
        }
        let Some(model) = entity.get_component::<ModelComponent>() else {
            return;
        };
        if let Some(placed) =
            self.placement
                .get(model.get_model(), parent_transform, &self.grip, self.hand)
        {
            self.model.render(scene, entity, view_projection, &placed);
        }
    }

    fn handle_event(&mut self, _: &mut Glfw, _: &mut glfw::Window, _: &WindowEvent) {}

    fn display_name(&self) -> String {
        "Held item".into()
    }
}
