use std::cell::{Cell, RefCell};

use cgmath::{Matrix4, Point3, SquareMatrix, Vector3};
use glfw::{Glfw, WindowEvent};

use crate::core::{
    entity::{
        component::{Component, Property, PropertyValue},
        Entity,
    },
    scene::{scene_file::ComponentData, Scene},
};

use super::*;

/// An effect on an entity. It runs in play mode and while the scene is
/// edited, so what it looks like can be tuned live.
pub struct ParticleEmitter {
    system: ParticleSystem,
    /// The entity's world transform when it was last drawn: where new
    /// particles start. None until it is drawn the first time.
    transform: Cell<Option<Matrix4<f32>>>,
    /// Removes its entity once its particles are gone (one-shot effects).
    pub despawn_when_done: bool,
    /// What `render` draws with: the particles sorted, packed.
    scratch: RefCell<Vec<[f32; 16]>>,
}

impl ParticleEmitter {
    pub fn new(effect: ParticleEffect) -> Self {
        // Different emitters of the same effect should not look the same.
        static SEED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x9E37_79B9);
        let seed = SEED.fetch_add(0x6D2B_79F5, std::sync::atomic::Ordering::Relaxed);
        Self {
            system: ParticleSystem::new(effect, seed),
            transform: Cell::new(None),
            despawn_when_done: false,
            scratch: RefCell::new(Vec::new()),
        }
    }

    pub fn effect(&self) -> &ParticleEffect {
        &self.system.effect
    }

    pub fn set_effect(&mut self, effect: ParticleEffect) {
        self.system.effect = effect.sanitized();
    }

    pub fn system(&self) -> &ParticleSystem {
        &self.system
    }

    /// Start over (a burst again).
    pub fn restart(&mut self) {
        self.system.restart();
    }

    fn step(&mut self, scene: &mut Scene, entity: &Entity, delta_time: f64) {
        // Not drawn yet: where it is in the world is not known.
        let Some(transform) = self.transform.get() else {
            return;
        };
        self.system.step(delta_time as f32, &transform);
        if self.despawn_when_done && self.system.finished() {
            let id = entity.id;
            self.despawn_when_done = false;
            scene.defer(move |scene| {
                scene.remove_entity(&id);
            });
        }
    }
}

/// A one-shot effect (a burst, not looping) at `position`: an entity that
/// removes itself when its particles are gone. Added once every entity is
/// updated.
pub fn spawn_burst(scene: &mut Scene, effect: ParticleEffect, position: Point3<f32>) {
    let mut emitter = ParticleEmitter::new(ParticleEffect {
        looping: false,
        ..effect
    });
    emitter.despawn_when_done = true;
    let mut entity = Entity::new("particles");
    entity.set_transient(true);
    entity.set_local_position(position);
    // Known at once, so it can start before it is first drawn.
    emitter
        .transform
        .set(Some(Matrix4::from_translation(Vector3::new(position.x, position.y, position.z))));
    entity.add_component(emitter);
    scene.defer(move |scene| scene.add_entity(entity));
}

/// A float field of an effect, as the inspector shows it.
type FloatField = (&'static str, f32, f32, fn(&ParticleEffect) -> f32, fn(&mut ParticleEffect, f32));

fn float_fields() -> Vec<FloatField> {
    fn shape_size(e: &ParticleEffect) -> f32 {
        e.shape.size()
    }
    fn set_shape_size(e: &mut ParticleEffect, v: f32) {
        e.shape = match e.shape {
            EmitterShape::Point => EmitterShape::Point,
            EmitterShape::Sphere(_) => EmitterShape::Sphere(v),
            EmitterShape::Disc(_) => EmitterShape::Disc(v),
            EmitterShape::Box(_) => EmitterShape::Box([v; 3]),
        }
    }
    vec![
        ("rate", 0.0, 500.0, |e| e.rate, |e, v| e.rate = v),
        ("burst", 0.0, 500.0, |e| e.burst as f32, |e, v| e.burst = v.max(0.0).round() as u32),
        ("duration (s)", 0.05, 60.0, |e| e.duration, |e, v| e.duration = v),
        ("max particles", 1.0, 5000.0, |e| e.max_particles as f32, |e, v| e.max_particles = v.max(1.0).round() as u32),
        ("lifetime min", 0.05, 30.0, |e| e.lifetime[0], |e, v| e.lifetime[0] = v),
        ("lifetime max", 0.05, 30.0, |e| e.lifetime[1], |e, v| e.lifetime[1] = v),
        ("shape size", 0.0, 20.0, shape_size, set_shape_size),
        ("direction x", -1.0, 1.0, |e| e.direction[0], |e, v| e.direction[0] = v),
        ("direction y", -1.0, 1.0, |e| e.direction[1], |e, v| e.direction[1] = v),
        ("direction z", -1.0, 1.0, |e| e.direction[2], |e, v| e.direction[2] = v),
        ("spread", 0.0, 180.0, |e| e.spread, |e, v| e.spread = v),
        ("speed min", 0.0, 20.0, |e| e.speed[0], |e, v| e.speed[0] = v),
        ("speed max", 0.0, 20.0, |e| e.speed[1], |e, v| e.speed[1] = v),
        ("gravity", -2.0, 2.0, |e| e.gravity, |e, v| e.gravity = v),
        ("wind x", -5.0, 5.0, |e| e.wind[0], |e, v| e.wind[0] = v),
        ("wind z", -5.0, 5.0, |e| e.wind[2], |e, v| e.wind[2] = v),
        ("drag", 0.0, 10.0, |e| e.drag, |e, v| e.drag = v),
        ("turbulence", 0.0, 10.0, |e| e.turbulence, |e, v| e.turbulence = v),
        ("size start", 0.0, 10.0, |e| e.size[0], |e, v| e.size[0] = v),
        ("size end", 0.0, 10.0, |e| e.size[1], |e, v| e.size[1] = v),
        ("size jitter", 0.0, 1.0, |e| e.size_jitter, |e, v| e.size_jitter = v),
        ("spin", 0.0, 20.0, |e| e.spin, |e, v| e.spin = v),
        ("start r", 0.0, 1.0, |e| e.color_start[0], |e, v| e.color_start[0] = v),
        ("start g", 0.0, 1.0, |e| e.color_start[1], |e, v| e.color_start[1] = v),
        ("start b", 0.0, 1.0, |e| e.color_start[2], |e, v| e.color_start[2] = v),
        ("start a", 0.0, 1.0, |e| e.color_start[3], |e, v| e.color_start[3] = v),
        ("middle r", 0.0, 1.0, |e| e.color_middle[0], |e, v| e.color_middle[0] = v),
        ("middle g", 0.0, 1.0, |e| e.color_middle[1], |e, v| e.color_middle[1] = v),
        ("middle b", 0.0, 1.0, |e| e.color_middle[2], |e, v| e.color_middle[2] = v),
        ("middle a", 0.0, 1.0, |e| e.color_middle[3], |e, v| e.color_middle[3] = v),
        ("end r", 0.0, 1.0, |e| e.color_end[0], |e, v| e.color_end[0] = v),
        ("end g", 0.0, 1.0, |e| e.color_end[1], |e, v| e.color_end[1] = v),
        ("end b", 0.0, 1.0, |e| e.color_end[2], |e, v| e.color_end[2] = v),
        ("end a", 0.0, 1.0, |e| e.color_end[3], |e, v| e.color_end[3] = v),
        ("emissive", 0.0, 30.0, |e| e.emissive, |e, v| e.emissive = v),
        ("stretch", 0.0, 0.2, |e| e.stretch, |e, v| e.stretch = v),
        ("soft", 0.0, 3.0, |e| e.soft, |e, v| e.soft = v),
    ]
}

impl Component for ParticleEmitter {
    fn update(&mut self, scene: &mut Scene, entity: &mut Entity, delta_time: f64) {
        self.step(scene, entity, delta_time);
    }

    fn update_in_editor(&mut self, scene: &mut Scene, entity: &mut Entity, delta_time: f64) {
        // A one-shot effect plays again and again while it is edited.
        if self.system.finished() && !self.despawn_when_done {
            self.system.restart();
        }
        self.step(scene, entity, delta_time);
    }

    fn render_transparent(
        &self,
        scene: &Scene,
        _: &Entity,
        view_projection: &Matrix4<f32>,
        parent_transform: &Matrix4<f32>,
    ) {
        self.transform.set(Some(*parent_transform));
        if self.system.particles.is_empty() {
            return;
        }
        let local = if self.system.effect.world_space {
            Matrix4::identity()
        } else {
            *parent_transform
        };
        render::draw(scene, &self.system, &local, view_projection, &mut self.scratch.borrow_mut());
    }

    fn handle_event(&mut self, _: &mut Glfw, _: &mut glfw::Window, _: &WindowEvent) {}

    fn display_name(&self) -> String {
        "Particles".into()
    }

    fn to_data(&self) -> Option<ComponentData> {
        // One-shot effects made by the game are not part of the scene.
        (!self.despawn_when_done).then(|| ComponentData::Particles {
            effect: self.system.effect,
        })
    }

    fn properties(&self) -> Vec<Property> {
        let e = &self.system.effect;
        let mut properties = vec![
            Property::choice(
                "preset",
                // The preset it is, until something is changed.
                ParticlePreset::ALL
                    .iter()
                    .find(|p| p.effect() == *e)
                    .map_or("Custom", |p| p.name())
                    .into(),
                std::iter::once("Custom".to_string())
                    .chain(ParticlePreset::ALL.iter().map(|p| p.name().to_string()))
                    .collect(),
            ),
            Property::bool("enabled", e.enabled),
            Property::bool("looping", e.looping),
            Property::choice(
                "shape",
                e.shape.name().into(),
                EmitterShape::NAMES.iter().map(|s| s.to_string()).collect(),
            ),
            Property::choice(
                "look",
                e.look.name().into(),
                ParticleLook::ALL.iter().map(|l| l.name().to_string()).collect(),
            ),
            Property::choice(
                "blend",
                e.blend.name().into(),
                ParticleBlend::ALL.iter().map(|b| b.name().to_string()).collect(),
            ),
            Property::bool("lit", e.lit),
            Property::bool("world space", e.world_space),
        ];
        for (name, min, max, get, _) in float_fields() {
            properties.push(Property::ranged(name, get(e), min, max));
        }
        properties.push(Property::int("alive", self.system.particles.len() as i64).read_only());
        properties
    }

    fn set_property(&mut self, name: &str, value: PropertyValue) {
        let mut e = self.system.effect;
        match name {
            "preset" => match value.as_text().and_then(ParticlePreset::from_name) {
                Some(preset) => {
                    e = preset.effect();
                    self.system.restart();
                }
                None => return,
            },
            "enabled" => e.enabled = value.as_bool().unwrap_or(e.enabled),
            "looping" => e.looping = value.as_bool().unwrap_or(e.looping),
            "lit" => e.lit = value.as_bool().unwrap_or(e.lit),
            "world space" => e.world_space = value.as_bool().unwrap_or(e.world_space),
            "shape" => {
                if let Some(shape) = value.as_text().and_then(|n| e.shape.renamed(n)) {
                    e.shape = shape;
                }
            }
            "look" => {
                if let Some(look) = value
                    .as_text()
                    .and_then(|n| ParticleLook::ALL.into_iter().find(|l| l.name().eq_ignore_ascii_case(n)))
                {
                    e.look = look;
                }
            }
            "blend" => {
                if let Some(blend) = value
                    .as_text()
                    .and_then(|n| ParticleBlend::ALL.into_iter().find(|b| b.name().eq_ignore_ascii_case(n)))
                {
                    e.blend = blend;
                }
            }
            _ => {
                let Some(v) = value.as_f32() else {
                    return;
                };
                match float_fields().into_iter().find(|(n, ..)| *n == name) {
                    Some((_, _, _, _, set)) => set(&mut e, v),
                    None => return,
                }
            }
        }
        let was_looping = self.system.effect.looping;
        self.set_effect(e);
        // Switched to looping after it stopped: start again.
        if self.system.effect.looping && !was_looping {
            self.system.restart();
        }
    }
}
