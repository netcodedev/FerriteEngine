//! The engine's own settings and knobs. High is what the engine did before
//! there were settings (docs/graphics-settings.md, "Settings (engine)").

use std::collections::BTreeMap;

use super::{EngineKnobs, Param, PerLevel, Resolved, SettingDef, SettingKind};

pub const VIEW: &str = "view";
pub const SHADOWS: &str = "shadows";
pub const VEGETATION: &str = "vegetation";
pub const EFFECTS: &str = "effects";

pub const MSAA: &str = "display.msaa";
pub const RESOLUTION_SCALE: &str = "display.resolution_scale";
pub const VSYNC: &str = "display.vsync";
pub const FPS_CAP: &str = "display.fps_cap";
pub const ANISOTROPY: &str = "display.anisotropy";
pub const TAA: &str = "display.taa";
pub const RAY_TRACED_SHADOWS: &str = "shadows.raytraced";
pub const RAY_TRACED_GRASS: &str = "shadows.raytraced_grass";
pub const RAY_TRACED_WIND: &str = "shadows.raytraced_wind";
pub const RAY_TRACED_LIGHTS: &str = "shadows.raytraced_lights";
/// Ambient occlusion: off, screen space, or ray traced.
pub const AMBIENT_OCCLUSION: &str = "effects.ao";
pub const AO_OPTIONS: [&str; 3] = ["off", "SSAO", "RT-AO"];
/// Reflections: the sky alone, the scene marched in screen space (water),
/// or ray traced (every glossy surface).
pub const REFLECTIONS: &str = "effects.reflections";
pub const REFLECTION_OPTIONS: [&str; 3] = ["Sky", "SSR", "RT"];

pub const MSAA_OPTIONS: [&str; 4] = ["off", "2x", "4x", "8x"];
pub const ANISOTROPY_OPTIONS: [&str; 4] = ["1x", "4x", "8x", "16x"];

/// The values before settings existed: what `Resolved::engine` is until
/// the settings are worked out.
pub const TODAY: EngineKnobs = EngineKnobs {
    msaa: 8,
    resolution_scale: 1.0,
    vsync: true,
    fps_cap: 0.0,
    anisotropy: 8,
    load_distance: 720.0,
    detail_distance: 300.0,
    terrain_lod_factor: 1.0,
    shadow_map_size: 4096,
    shadow_distance: 300.0,
    shadow_cascades: 4,
    stagger_second_cascade: false,
    point_shadow_resolution: 512,
    point_shadow_lights: 8,
    scatter_lod_factor: 1.0,
    detail_distance_factor: 1.0,
    detail_density: 1.0,
    ssao_samples: 16,
    taa: false,
    ray_traced_shadows: false,
    ray_traced_grass: false,
    ray_traced_wind: false,
    ray_traced_lights: false,
    ambient_occlusion: super::AoMode::Ssao,
    reflections: super::ReflectionMode::Ssr,
    bloom: true,
    cloud_downscale: 2,
    precipitation: 1.0,
};

fn level(key: &str, label: &str, group: &str, description: &str) -> SettingDef {
    SettingDef {
        key: key.into(),
        label: label.into(),
        group: group.into(),
        kind: SettingKind::Level,
        description: description.into(),
    }
}

fn choice(key: &str, label: &str, options: &[&str], per_preset: [&str; 4], description: &str) -> SettingDef {
    let [low, medium, high, ultra] = per_preset.map(String::from);
    SettingDef {
        key: key.into(),
        label: label.into(),
        group: "Anzeige".into(),
        kind: SettingKind::Choice {
            options: options.iter().map(|o| o.to_string()).collect(),
            per_preset: PerLevel { low, medium, high, ultra },
        },
        description: description.into(),
    }
}

/// The engine's settings, in menu order.
pub fn engine_definitions() -> Vec<SettingDef> {
    vec![
        choice(MSAA, "Kantenglättung (MSAA)", &MSAA_OPTIONS, ["off", "4x", "8x", "8x"],
            "Glättet Kanten. Höher kostet vor allem Grafikspeicher und Füllrate."),
        SettingDef {
            key: TAA.into(),
            label: "Temporale Kantenglättung (TAA)".into(),
            group: "Anzeige".into(),
            kind: SettingKind::Toggle { per_preset: PerLevel::all(false) },
            description: "Glättet Kanten über mehrere Bilder, auch an Gras und Blättern. Ersetzt MSAA, solange es an ist.".into(),
        },
        SettingDef {
            key: RESOLUTION_SCALE.into(),
            label: "Auflösungsskala".into(),
            group: "Anzeige".into(),
            kind: SettingKind::Number {
                min: 50.0,
                max: 100.0,
                step: 5.0,
                unit: "%".into(),
                per_preset: PerLevel::new(75.0, 100.0, 100.0, 100.0),
            },
            description: "Die Szene wird kleiner gezeichnet und hochskaliert.".into(),
        },
        SettingDef {
            key: VSYNC.into(),
            label: "VSync".into(),
            group: "Anzeige".into(),
            kind: SettingKind::Toggle { per_preset: PerLevel::all(true) },
            description: "Auf den Bildschirm warten: kein Tearing.".into(),
        },
        SettingDef {
            key: FPS_CAP.into(),
            label: "FPS-Grenze".into(),
            group: "Anzeige".into(),
            kind: SettingKind::Number {
                min: 0.0,
                max: 240.0,
                step: 10.0,
                unit: "fps".into(),
                per_preset: PerLevel::all(0.0),
            },
            description: "Höchstens so viele Bilder pro Sekunde; 0: keine Grenze.".into(),
        },
        choice(ANISOTROPY, "Texturfilterung", &ANISOTROPY_OPTIONS, ["1x", "4x", "8x", "16x"],
            "Anisotrope Filterung: schärfere Böden in flachem Winkel."),
        level(VIEW, "Sichtweite", "Welt", "Wie weit Terrain geladen und fein gezeichnet wird."),
        level(VEGETATION, "Vegetation", "Welt", "Detailstufen von Bäumen und Steinen, Reichweite und Dichte von Gras."),
        level(SHADOWS, "Schatten", "Licht", "Auflösung und Reichweite der Schatten."),
        SettingDef {
            key: RAY_TRACED_SHADOWS.into(),
            label: "Raytracing-Schatten".into(),
            group: "Licht".into(),
            kind: SettingKind::Toggle { per_preset: PerLevel::all(false) },
            description: "Sonnenschatten per Raytracing statt Schattenkarten: weich, ohne Reichweitengrenze. Nur mit Vulkan auf Grafikkarten mit Raytracing.".into(),
        },
        SettingDef {
            key: RAY_TRACED_GRASS.into(),
            label: "Raytracing-Schatten von Gras".into(),
            group: "Licht".into(),
            kind: SettingKind::Toggle { per_preset: PerLevel::all(false) },
            description: "Gras in der Nähe wirft Raytracing-Schatten. Kostet bei dichtem Gras viel; nur mit Raytracing-Schatten.".into(),
        },
        SettingDef {
            key: RAY_TRACED_WIND.into(),
            label: "Raytracing-Schatten im Wind".into(),
            group: "Licht".into(),
            kind: SettingKind::Toggle { per_preset: PerLevel::all(false) },
            description: "Die Lichtflecken im Schatten von Laub bewegen sich mit dem Wind (die Umrisse der Kronen bleiben stehen); nur mit Raytracing-Schatten.".into(),
        },
        SettingDef {
            key: RAY_TRACED_LIGHTS.into(),
            label: "Raytracing-Lampenschatten".into(),
            group: "Licht".into(),
            kind: SettingKind::Toggle { per_preset: PerLevel::all(false) },
            description: "Schatten von Lampen per Raytracing (ReSTIR) statt Schattenwürfeln: weich, für jede Lampe mit Schatten, nicht nur die acht nächsten. Nur mit Vulkan auf Grafikkarten mit Raytracing.".into(),
        },
        SettingDef {
            key: AMBIENT_OCCLUSION.into(),
            label: "Umgebungsverdeckung".into(),
            group: "Licht".into(),
            kind: SettingKind::Choice {
                options: AO_OPTIONS.iter().map(|o| o.to_string()).collect(),
                per_preset: PerLevel {
                    low: "off".into(),
                    medium: "SSAO".into(),
                    high: "SSAO".into(),
                    ultra: "SSAO".into(),
                },
            },
            description: "Dunkelt Ecken und Spalten ab. SSAO rechnet im Bild; RT-AO per Raytracing, auch mit dem, was außerhalb des Bildes oder dahinter liegt (nur mit Vulkan auf Grafikkarten mit Raytracing, sonst SSAO). Radius und Stärke stellt das Himmelslicht ein.".into(),
        },
        SettingDef {
            key: REFLECTIONS.into(),
            label: "Reflexionen".into(),
            group: "Licht".into(),
            kind: SettingKind::Choice {
                options: REFLECTION_OPTIONS.iter().map(|o| o.to_string()).collect(),
                per_preset: PerLevel::all("SSR".to_string()),
            },
            description: "Was glänzende Flächen spiegeln. Sky: nur den Himmel. SSR: Wasser spiegelt, was im Bild ist. RT: Wasser, Pfützen, Lack, Glas und Metall spiegeln die Szene per Raytracing, auch was außerhalb des Bildes liegt (nur mit Vulkan auf Grafikkarten mit Raytracing, sonst SSR).".into(),
        },
        level(EFFECTS, "Effekte", "Licht", "Umgebungsverdeckung, Bloom, Wolken und Niederschlag."),
    ]
}

fn knob(setting: &str, low: f32, medium: f32, high: f32, ultra: f32) -> Param<f32> {
    Param::ByLevel { setting: setting.into(), values: PerLevel::new(low, medium, high, ultra) }
}

/// The engine's knobs: each a value per level of a group's setting.
/// Toggles are 0 or 1.
pub fn engine_knobs() -> BTreeMap<String, Param<f32>> {
    [
        ("terrain.load_distance", knob(VIEW, 400.0, 560.0, 720.0, 1350.0)),
        ("terrain.detail_distance", knob(VIEW, 200.0, 250.0, 300.0, 450.0)),
        ("terrain.lod_factor", knob(VIEW, 0.6, 0.8, 1.0, 1.5)),
        ("shadows.map_size", knob(SHADOWS, 1024.0, 2048.0, 4096.0, 4096.0)),
        ("shadows.distance", knob(SHADOWS, 150.0, 220.0, 300.0, 450.0)),
        ("shadows.cascades", knob(SHADOWS, 2.0, 3.0, 4.0, 4.0)),
        ("shadows.stagger_second", knob(SHADOWS, 1.0, 1.0, 0.0, 0.0)),
        ("shadows.point_resolution", knob(SHADOWS, 256.0, 512.0, 512.0, 1024.0)),
        ("shadows.point_lights", knob(SHADOWS, 2.0, 4.0, 8.0, 8.0)),
        ("scatter.lod_factor", knob(VEGETATION, 0.6, 0.8, 1.0, 1.5)),
        ("scatter.detail_distance_factor", knob(VEGETATION, 0.5, 0.75, 1.0, 1.4)),
        ("scatter.detail_density", knob(VEGETATION, 0.5, 0.75, 1.0, 1.0)),
        ("effects.ssao_samples", knob(EFFECTS, 8.0, 8.0, 16.0, 16.0)),
        ("effects.bloom", knob(EFFECTS, 0.0, 1.0, 1.0, 1.0)),
        ("effects.cloud_downscale", knob(EFFECTS, 4.0, 2.0, 2.0, 1.0)),
        ("effects.precipitation", knob(EFFECTS, 0.3, 0.6, 1.0, 1.0)),
    ]
    .into_iter()
    .map(|(key, param)| (key.to_string(), param))
    .collect()
}

/// Parse `"4x"` into 4 (`"off"` into 1).
fn times(option: Option<&str>, fallback: u32) -> u32 {
    match option {
        Some("off") => 1,
        Some(o) => o.trim_end_matches('x').parse().unwrap_or(fallback),
        None => fallback,
    }
}

/// The typed knobs from `resolved`'s settings and knobs, each kept to
/// what the engine can do.
pub fn engine_values(resolved: &Resolved) -> EngineKnobs {
    let t = TODAY;
    let k = |key: &str, fallback: f32| resolved.knob(key).filter(|v| v.is_finite()).unwrap_or(fallback);
    let load = k("terrain.load_distance", t.load_distance).clamp(128.0, MAX_LOAD_DISTANCE);
    EngineKnobs {
        msaa: times(resolved.choice(MSAA), t.msaa).clamp(1, 8),
        resolution_scale: (resolved.number(RESOLUTION_SCALE).unwrap_or(100.0) / 100.0).clamp(0.25, 1.0),
        vsync: resolved.toggle(VSYNC).unwrap_or(t.vsync),
        fps_cap: resolved.number(FPS_CAP).unwrap_or(0.0).max(0.0),
        anisotropy: times(resolved.choice(ANISOTROPY), t.anisotropy as u32).clamp(1, 16) as u8,
        load_distance: load,
        detail_distance: k("terrain.detail_distance", t.detail_distance).clamp(0.0, load),
        terrain_lod_factor: k("terrain.lod_factor", 1.0).clamp(0.1, 10.0),
        shadow_map_size: (k("shadows.map_size", 4096.0).round() as u32).clamp(256, 8192),
        shadow_distance: k("shadows.distance", t.shadow_distance).clamp(20.0, 2000.0),
        shadow_cascades: (k("shadows.cascades", 4.0).round() as usize).clamp(1, 4),
        stagger_second_cascade: k("shadows.stagger_second", 0.0) >= 0.5,
        point_shadow_resolution: (k("shadows.point_resolution", 512.0).round() as u32).clamp(64, 4096),
        point_shadow_lights: (k("shadows.point_lights", 8.0).round() as usize).clamp(0, 8),
        scatter_lod_factor: k("scatter.lod_factor", 1.0).clamp(0.1, 10.0),
        detail_distance_factor: k("scatter.detail_distance_factor", 1.0).clamp(0.1, 10.0),
        detail_density: k("scatter.detail_density", 1.0).clamp(0.0, 1.0),
        ssao_samples: (k("effects.ssao_samples", 16.0).round() as u32).clamp(0, 16),
        taa: resolved.toggle(TAA).unwrap_or(t.taa),
        ray_traced_shadows: resolved.toggle(RAY_TRACED_SHADOWS).unwrap_or(t.ray_traced_shadows),
        ray_traced_grass: resolved.toggle(RAY_TRACED_GRASS).unwrap_or(t.ray_traced_grass),
        ray_traced_wind: resolved.toggle(RAY_TRACED_WIND).unwrap_or(t.ray_traced_wind),
        ray_traced_lights: resolved.toggle(RAY_TRACED_LIGHTS).unwrap_or(t.ray_traced_lights),
        reflections: match resolved.choice(REFLECTIONS) {
            Some("Sky") => super::ReflectionMode::Sky,
            Some("RT") => super::ReflectionMode::RayTraced,
            Some(_) => super::ReflectionMode::Ssr,
            None => t.reflections,
        },
        ambient_occlusion: match resolved.choice(AMBIENT_OCCLUSION) {
            Some("off") => super::AoMode::Off,
            Some("RT-AO") => super::AoMode::RayTraced,
            Some(_) => super::AoMode::Ssao,
            None => t.ambient_occlusion,
        },
        bloom: k("effects.bloom", 1.0) >= 0.5,
        cloud_downscale: (k("effects.cloud_downscale", 2.0).round() as i32).clamp(1, 8),
        precipitation: k("effects.precipitation", 1.0).clamp(0.0, 1.0),
    }
}

/// The farthest any level may load terrain: the loaded-columns picture
/// (terrain/columns.rs) reaches 12 columns of 128 m, and a column loads
/// until `load + 128`.
pub const MAX_LOAD_DISTANCE: f32 = 12.0 * 128.0 - 128.0 - 32.0;
