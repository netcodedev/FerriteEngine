use cgmath::{InnerSpace, Matrix4, Vector3};

use crate::core::{entity::component::{Component, PropertyValue}, scene::scene_file::ComponentData};

use super::*;

fn quiet(effect: ParticleEffect) -> ParticleSystem {
    ParticleSystem::new(effect, 12345)
}

fn at(x: f32, y: f32, z: f32) -> Matrix4<f32> {
    Matrix4::from_translation(Vector3::new(x, y, z))
}

#[test]
fn particles_come_at_the_rate_asked_for() {
    let mut s = quiet(ParticleEffect {
        rate: 10.0,
        lifetime: [100.0, 100.0],
        max_particles: 1000,
        ..ParticlePreset::Smoke.effect()
    });
    for _ in 0..100 {
        s.step(0.01, &at(0.0, 0.0, 0.0));
    }
    // One second: ten, give or take the one owed.
    assert!((9..=10).contains(&s.particles.len()), "{}", s.particles.len());
}

#[test]
fn a_burst_comes_at_once_and_a_one_shot_finishes() {
    let mut s = quiet(ParticlePreset::Sparks.effect());
    s.step(0.016, &at(0.0, 0.0, 0.0));
    assert_eq!(s.particles.len(), 40);
    assert!(!s.finished());
    for _ in 0..200 {
        s.step(0.016, &at(0.0, 0.0, 0.0));
    }
    assert!(s.finished());
    s.restart();
    s.step(0.016, &at(0.0, 0.0, 0.0));
    assert_eq!(s.particles.len(), 40);
}

#[test]
fn never_more_than_the_maximum() {
    let mut s = quiet(ParticleEffect {
        rate: 1000.0,
        burst: 500,
        max_particles: 50,
        lifetime: [10.0, 10.0],
        ..ParticlePreset::Fire.effect()
    });
    for _ in 0..10 {
        s.step(0.05, &at(0.0, 0.0, 0.0));
    }
    assert_eq!(s.particles.len(), 50);
}

#[test]
fn particles_die_at_the_end_of_their_life() {
    let mut s = quiet(ParticleEffect {
        rate: 0.0,
        burst: 5,
        lifetime: [0.5, 0.5],
        ..ParticlePreset::Embers.effect()
    });
    s.step(0.01, &at(0.0, 0.0, 0.0));
    assert_eq!(s.particles.len(), 5);
    for _ in 0..60 {
        s.step(0.01, &at(0.0, 0.0, 0.0));
    }
    assert!(s.particles.is_empty());
}

#[test]
fn gravity_pulls_them_down_and_negative_gravity_lifts_them() {
    for (gravity, down) in [(1.0, true), (-1.0, false)] {
        let mut s = quiet(ParticleEffect {
            rate: 0.0,
            burst: 1,
            speed: [0.0, 0.0],
            gravity,
            drag: 0.0,
            turbulence: 0.0,
            wind: [0.0; 3],
            lifetime: [5.0, 5.0],
            shape: EmitterShape::Point,
            ..ParticlePreset::Dust.effect()
        });
        for _ in 0..50 {
            s.step(0.01, &at(0.0, 10.0, 0.0));
        }
        let y = s.particles[0].position.y;
        // Half a second: about 1.2 m.
        assert!((y - 10.0).abs() > 1.0, "{y}");
        assert_eq!(y < 10.0, down);
    }
}

#[test]
fn they_start_inside_the_shape_around_the_emitter() {
    for shape in [
        EmitterShape::Sphere(2.0),
        EmitterShape::Box([1.0, 0.5, 3.0]),
        EmitterShape::Disc(1.5),
    ] {
        let mut s = quiet(ParticleEffect {
            rate: 0.0,
            burst: 200,
            max_particles: 200,
            speed: [0.0, 0.0],
            shape,
            ..ParticlePreset::Dust.effect()
        });
        s.step(0.0, &at(10.0, 20.0, 30.0));
        for p in &s.particles {
            let d = p.position - Vector3::new(10.0, 20.0, 30.0);
            let inside = match shape {
                EmitterShape::Sphere(r) => d.magnitude() <= r + 1e-4,
                EmitterShape::Box([x, y, z]) => d.x.abs() <= x && d.y.abs() <= y && d.z.abs() <= z,
                EmitterShape::Disc(r) => d.y.abs() < 1e-4 && (d.x * d.x + d.z * d.z).sqrt() <= r + 1e-4,
                EmitterShape::Point => true,
            };
            assert!(inside, "{shape:?}: {d:?}");
        }
    }
}

#[test]
fn they_fly_within_the_spread_of_the_direction() {
    for _ in 0..100 {
        let a = (0.37f32 * 7.0).fract();
        let d = cone(Vector3::unit_y(), 20f32.to_radians(), a, 0.6);
        assert!(d.dot(Vector3::unit_y()) >= 20f32.to_radians().cos() - 1e-5);
        assert!((d.magnitude() - 1.0).abs() < 1e-5);
    }
    let mut s = quiet(ParticleEffect {
        rate: 0.0,
        burst: 100,
        spread: 15.0,
        speed: [1.0, 1.0],
        ..ParticlePreset::Embers.effect()
    });
    // The entity turned upside down: they fly down.
    let flipped = Matrix4::from_angle_x(cgmath::Deg(180.0));
    s.step(0.0, &flipped);
    for p in &s.particles {
        assert!(p.velocity.normalize().y < -15f32.to_radians().cos() + 1e-4);
    }
}

#[test]
fn colour_and_size_change_over_their_life() {
    let e = ParticlePreset::Fire.effect();
    let near = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-5);
    assert!(near(e.color_at(0.0), e.color_start));
    assert!(near(e.color_at(0.5), e.color_middle));
    assert!(near(e.color_at(1.0), e.color_end));
    let quarter = e.color_at(0.25);
    assert!((quarter[3] - (e.color_start[3] + e.color_middle[3]) / 2.0).abs() < 1e-6);
    assert!((e.size_at(0.5) - (e.size[0] + e.size[1]) / 2.0).abs() < 1e-6);
}

#[test]
fn hand_edited_effects_are_put_in_range() {
    let e = ParticleEffect {
        rate: -5.0,
        lifetime: [3.0, f32::NAN],
        speed: [5.0, 1.0],
        spread: 400.0,
        max_particles: 0,
        direction: [0.0, 0.0, 0.0],
        color_start: [2.0, -1.0, 0.5, 1.0],
        ..ParticlePreset::Fire.effect()
    }
    .sanitized();
    assert_eq!(e.rate, 0.0);
    assert!(e.lifetime[0] <= e.lifetime[1]);
    assert_eq!(e.speed, [1.0, 5.0]);
    assert_eq!(e.spread, 180.0);
    assert_eq!(e.max_particles, 1);
    assert_eq!(e.direction, [0.0, 1.0, 0.0]);
    assert_eq!(e.color_start, [1.0, 0.0, 0.5, 1.0]);
}

#[test]
fn every_preset_is_in_range_and_has_a_name() {
    for preset in ParticlePreset::ALL {
        let e = preset.effect();
        assert_eq!(e.sanitized(), e, "{}", preset.name());
        assert_eq!(ParticlePreset::from_name(preset.name()), Some(preset));
    }
}

#[test]
fn effects_round_trip_through_ron_and_may_leave_out_fields() {
    let e = ParticlePreset::Smoke.effect();
    let text = ron::to_string(&e).unwrap();
    assert_eq!(ron::from_str::<ParticleEffect>(&text).unwrap(), e);
    let short: ParticleEffect = ron::from_str("(rate: 3.0, shape: Sphere(1.0))").unwrap();
    assert_eq!(short.rate, 3.0);
    assert_eq!(short.shape, EmitterShape::Sphere(1.0));
    assert_eq!(short.look, ParticleEffect::default().look);
}

#[test]
fn the_inspector_reads_and_writes_every_field() {
    let mut emitter = ParticleEmitter::new(ParticlePreset::Fire.effect());
    emitter.set_property("rate", PropertyValue::Float(12.0));
    emitter.set_property("end a", PropertyValue::Float(0.5));
    emitter.set_property("shape", PropertyValue::Text("box".into()));
    emitter.set_property("blend", PropertyValue::Text("Alpha".into()));
    let e = *emitter.effect();
    assert_eq!(e.rate, 12.0);
    assert_eq!(e.color_end[3], 0.5);
    assert!(matches!(e.shape, EmitterShape::Box(_)));
    assert_eq!(e.blend, ParticleBlend::Alpha);
    emitter.set_property("preset", PropertyValue::Text("Smoke".into()));
    assert_eq!(*emitter.effect(), ParticlePreset::Smoke.effect());
    let shown = |emitter: &ParticleEmitter| {
        emitter.properties().into_iter().find(|p| p.name == "preset").unwrap().value
    };
    assert_eq!(shown(&emitter), PropertyValue::Text("Smoke".into()));
    // Every float it shows it takes back unchanged.
    for p in emitter.properties() {
        if p.read_only {
            continue;
        }
        let before = *emitter.effect();
        emitter.set_property(&p.name, p.value.clone());
        assert_eq!(*emitter.effect(), before, "{}", p.name);
    }
}

#[test]
fn it_saves_as_its_effect_but_one_shots_do_not() {
    let mut emitter = ParticleEmitter::new(ParticlePreset::Embers.effect());
    assert_eq!(
        emitter.to_data(),
        Some(ComponentData::Particles {
            effect: ParticlePreset::Embers.effect()
        })
    );
    emitter.despawn_when_done = true;
    assert_eq!(emitter.to_data(), None);
}

#[test]
fn a_moving_emitter_leaves_world_space_particles_behind() {
    let mut s = quiet(ParticleEffect {
        rate: 0.0,
        burst: 1,
        speed: [0.0, 0.0],
        gravity: 0.0,
        turbulence: 0.0,
        wind: [0.0; 3],
        shape: EmitterShape::Point,
        ..ParticlePreset::Smoke.effect()
    });
    s.step(0.01, &at(1.0, 2.0, 3.0));
    s.step(0.01, &at(50.0, 0.0, 0.0));
    assert!((s.particles[0].position - Vector3::new(1.0, 2.0, 3.0)).magnitude() < 1e-4);
}
