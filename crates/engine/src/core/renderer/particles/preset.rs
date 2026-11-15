use serde::{Deserialize, Serialize};

use super::*;

/// Ready-made effects, as the editor offers them.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticlePreset {
    Fire,
    Smoke,
    Embers,
    Sparks,
    Dust,
    WoodChips,
    Fireflies,
    Mist,
}

impl ParticlePreset {
    pub const ALL: [ParticlePreset; 8] = [
        ParticlePreset::Fire,
        ParticlePreset::Smoke,
        ParticlePreset::Embers,
        ParticlePreset::Sparks,
        ParticlePreset::Dust,
        ParticlePreset::WoodChips,
        ParticlePreset::Fireflies,
        ParticlePreset::Mist,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ParticlePreset::Fire => "Fire",
            ParticlePreset::Smoke => "Smoke",
            ParticlePreset::Embers => "Embers",
            ParticlePreset::Sparks => "Sparks",
            ParticlePreset::Dust => "Dust",
            ParticlePreset::WoodChips => "Wood chips",
            ParticlePreset::Fireflies => "Fireflies",
            ParticlePreset::Mist => "Mist",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name().eq_ignore_ascii_case(name))
    }

    pub fn effect(self) -> ParticleEffect {
        let base = ParticleEffect {
            enabled: true,
            rate: 20.0,
            burst: 0,
            looping: true,
            duration: 5.0,
            max_particles: 400,
            lifetime: [1.0, 2.0],
            shape: EmitterShape::Point,
            direction: [0.0, 1.0, 0.0],
            spread: 20.0,
            speed: [0.5, 1.0],
            gravity: 0.0,
            wind: [0.0; 3],
            drag: 0.5,
            turbulence: 0.0,
            size: [0.3, 0.3],
            size_jitter: 0.2,
            spin: 0.5,
            color_start: [1.0; 4],
            color_middle: [1.0; 4],
            color_end: [1.0, 1.0, 1.0, 0.0],
            emissive: 0.0,
            lit: true,
            look: ParticleLook::Soft,
            blend: ParticleBlend::Alpha,
            stretch: 0.0,
            soft: 0.3,
            world_space: true,
        };
        match self {
            // Flames: quick, rising, yellow-white turning orange then a
            // dark red as they thin out.
            ParticlePreset::Fire => ParticleEffect {
                rate: 70.0,
                max_particles: 160,
                lifetime: [0.45, 0.9],
                shape: EmitterShape::Disc(0.28),
                spread: 12.0,
                speed: [0.4, 0.9],
                gravity: -0.25,
                drag: 1.2,
                turbulence: 1.4,
                size: [0.42, 0.12],
                size_jitter: 0.3,
                spin: 1.5,
                color_start: [1.0, 0.85, 0.45, 0.0],
                color_middle: [1.0, 0.45, 0.1, 0.8],
                color_end: [0.6, 0.08, 0.02, 0.0],
                emissive: 5.0,
                lit: false,
                look: ParticleLook::Smoke,
                blend: ParticleBlend::Additive,
                soft: 0.2,
                ..base
            },
            // Grey smoke rising, spreading and drifting off.
            ParticlePreset::Smoke => ParticleEffect {
                rate: 7.0,
                max_particles: 120,
                lifetime: [4.0, 7.0],
                shape: EmitterShape::Disc(0.2),
                spread: 10.0,
                speed: [0.6, 1.0],
                gravity: -0.05,
                wind: [0.25, 0.0, 0.1],
                drag: 0.4,
                turbulence: 0.35,
                size: [0.5, 2.6],
                size_jitter: 0.3,
                spin: 0.3,
                color_start: [0.35, 0.33, 0.32, 0.0],
                color_middle: [0.45, 0.44, 0.44, 0.45],
                color_end: [0.55, 0.55, 0.56, 0.0],
                look: ParticleLook::Smoke,
                blend: ParticleBlend::Alpha,
                soft: 0.6,
                ..base
            },
            // Glowing bits flying up out of a fire, swirling.
            ParticlePreset::Embers => ParticleEffect {
                rate: 9.0,
                max_particles: 60,
                lifetime: [1.2, 2.6],
                shape: EmitterShape::Disc(0.3),
                spread: 25.0,
                speed: [1.0, 2.4],
                gravity: -0.03,
                drag: 0.7,
                turbulence: 2.2,
                size: [0.035, 0.015],
                size_jitter: 0.3,
                color_start: [1.0, 0.75, 0.35, 1.0],
                color_middle: [1.0, 0.45, 0.12, 1.0],
                color_end: [0.8, 0.15, 0.05, 0.0],
                emissive: 8.0,
                lit: false,
                look: ParticleLook::Spark,
                blend: ParticleBlend::Additive,
                stretch: 0.04,
                soft: 0.0,
                ..base
            },
            // A shower of hot sparks, falling.
            ParticlePreset::Sparks => ParticleEffect {
                rate: 0.0,
                burst: 40,
                looping: false,
                duration: 0.1,
                max_particles: 80,
                lifetime: [0.3, 0.8],
                spread: 60.0,
                speed: [2.0, 5.0],
                gravity: 1.0,
                drag: 0.4,
                size: [0.03, 0.01],
                color_start: [1.0, 0.95, 0.7, 1.0],
                color_middle: [1.0, 0.6, 0.2, 1.0],
                color_end: [0.9, 0.25, 0.05, 0.0],
                emissive: 12.0,
                lit: false,
                look: ParticleLook::Spark,
                blend: ParticleBlend::Additive,
                stretch: 0.03,
                soft: 0.0,
                ..base
            },
            // A puff of dust that settles: building, landing.
            ParticlePreset::Dust => ParticleEffect {
                rate: 0.0,
                burst: 24,
                looping: false,
                duration: 0.1,
                max_particles: 40,
                lifetime: [1.2, 2.2],
                shape: EmitterShape::Box([1.2, 0.3, 1.2]),
                spread: 75.0,
                speed: [0.6, 1.6],
                gravity: 0.02,
                drag: 1.8,
                turbulence: 0.3,
                size: [0.5, 1.4],
                size_jitter: 0.35,
                color_start: [0.62, 0.56, 0.48, 0.55],
                color_middle: [0.66, 0.6, 0.52, 0.35],
                color_end: [0.7, 0.65, 0.58, 0.0],
                look: ParticleLook::Smoke,
                soft: 0.4,
                ..base
            },
            // Chips flying off a trunk an axe hits, tumbling down.
            ParticlePreset::WoodChips => ParticleEffect {
                rate: 0.0,
                burst: 26,
                looping: false,
                duration: 0.1,
                max_particles: 40,
                lifetime: [0.8, 1.6],
                shape: EmitterShape::Sphere(0.15),
                spread: 75.0,
                speed: [1.5, 3.5],
                gravity: 1.0,
                drag: 0.6,
                size: [0.07, 0.06],
                size_jitter: 0.4,
                spin: 9.0,
                color_start: [0.78, 0.62, 0.4, 1.0],
                color_middle: [0.72, 0.56, 0.36, 1.0],
                color_end: [0.6, 0.46, 0.3, 0.0],
                look: ParticleLook::Smoke,
                soft: 0.0,
                ..base
            },
            // Little green-yellow lights drifting about over a wide area.
            ParticlePreset::Fireflies => ParticleEffect {
                rate: 7.0,
                max_particles: 80,
                lifetime: [5.0, 9.0],
                shape: EmitterShape::Box([8.0, 1.2, 8.0]),
                spread: 180.0,
                speed: [0.1, 0.3],
                drag: 0.8,
                turbulence: 0.6,
                size: [0.13, 0.13],
                size_jitter: 0.2,
                spin: 0.0,
                color_start: [0.75, 1.0, 0.35, 0.0],
                color_middle: [0.8, 1.0, 0.4, 1.0],
                color_end: [0.7, 1.0, 0.3, 0.0],
                emissive: 7.0,
                lit: false,
                blend: ParticleBlend::Additive,
                soft: 0.0,
                ..base
            },
            // Low mist creeping over the ground or water.
            ParticlePreset::Mist => ParticleEffect {
                rate: 3.0,
                max_particles: 80,
                lifetime: [10.0, 16.0],
                shape: EmitterShape::Box([10.0, 0.3, 10.0]),
                spread: 180.0,
                speed: [0.05, 0.15],
                wind: [0.05, 0.0, 0.02],
                drag: 0.2,
                turbulence: 0.05,
                size: [3.0, 5.0],
                size_jitter: 0.3,
                spin: 0.05,
                color_start: [0.8, 0.82, 0.85, 0.0],
                color_middle: [0.8, 0.82, 0.85, 0.22],
                color_end: [0.8, 0.82, 0.85, 0.0],
                look: ParticleLook::Smoke,
                soft: 1.5,
                ..base
            },
        }
    }
}
