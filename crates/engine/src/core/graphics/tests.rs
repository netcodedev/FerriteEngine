use super::*;

fn project_with_toggle() -> ProjectGraphics {
    ProjectGraphics {
        default_preset: Level::Medium,
        settings: vec![SettingDef {
            key: "demo.blood".into(),
            label: "Blut".into(),
            group: "Effekte".into(),
            kind: SettingKind::Toggle { per_preset: PerLevel::new(false, true, true, true) },
            description: String::new(),
        }],
        hidden: vec![defs::FPS_CAP.into()],
        knobs: [("shadows.map_size".to_string(), Param::ByLevel {
            setting: "shadows".into(),
            values: PerLevel::new(512.0, 1024.0, 2048.0, 8192.0),
        })]
        .into_iter()
        .collect(),
    }
}

#[test]
fn high_is_what_the_engine_did_before() {
    let graphics = Graphics::default();
    let resolved = graphics.resolved();
    assert_eq!(resolved.preset, Level::High);
    assert_eq!(resolved.engine, defs::TODAY);
}

#[test]
fn params_round_trip_through_ron_in_both_forms() {
    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct Holder {
        distance: Param<f32>,
        shadows: Param<bool>,
        bound: Param<f32>,
        bound_bool: Param<bool>,
    }
    let holder = Holder {
        distance: Param::Fixed(60.0),
        shadows: Param::Fixed(true),
        bound: Param::ByLevel { setting: "vegetation".into(), values: PerLevel::new(30.0, 45.0, 60.0, 90.0) },
        bound_bool: Param::ByLevel { setting: "shadows".into(), values: PerLevel::new(false, true, true, true) },
    };
    let text = ron::ser::to_string_pretty(&holder, ron::ser::PrettyConfig::default()).unwrap();
    assert!(text.contains("distance: 60.0"), "{text}");
    assert_eq!(ron::from_str::<Holder>(&text).unwrap(), holder);
    // Old files: plain numbers, integers too.
    let old: Holder = ron::from_str(
        "(distance: 60, shadows: false, bound: 1.5, bound_bool: (setting: \"x\", low: true, medium: true, high: false, ultra: false))",
    )
    .unwrap();
    assert_eq!(old.distance, Param::Fixed(60.0));
    assert_eq!(old.shadows, Param::Fixed(false));
    assert_eq!(old.bound_bool.high(), false);
    // And through JSON (MCP).
    let json = serde_json::to_string(&holder).unwrap();
    assert_eq!(serde_json::from_str::<Holder>(&json).unwrap(), holder);
    // A level table without all levels is refused.
    assert!(ron::from_str::<Param<f32>>("(setting: \"x\", low: 1.0)").is_err());
}

#[test]
fn a_bound_param_follows_its_settings_level() {
    let mut graphics = Graphics::default();
    let param = Param::ByLevel { setting: "vegetation".into(), values: PerLevel::new(30.0, 45.0, 60.0, 90.0) };
    assert_eq!(param.resolve(&graphics.resolved()), 60.0);
    graphics.set("vegetation", SettingValue::Level(Level::Low)).unwrap();
    assert_eq!(param.resolve(&graphics.resolved()), 30.0);
    // Another group's level does not matter.
    graphics.set("shadows", SettingValue::Level(Level::Ultra)).unwrap();
    assert_eq!(param.resolve(&graphics.resolved()), 30.0);
    // Unknown setting: High.
    let unknown = Param::ByLevel { setting: "nope".into(), values: PerLevel::new(1.0, 2.0, 3.0, 4.0) };
    assert_eq!(unknown.resolve(&graphics.resolved()), 3.0);
}

#[test]
fn presets_set_everything_and_a_change_makes_custom() {
    let mut graphics = Graphics::default();
    graphics.set_preset(Level::Low);
    let low = graphics.resolved();
    assert!(!low.custom);
    assert_eq!(low.level("shadows"), Level::Low);
    assert_eq!(low.choice(defs::MSAA), Some("off"));
    assert_eq!(low.engine.shadow_map_size, 1024);
    assert_eq!(low.engine.msaa, 1);

    graphics.set("shadows", SettingValue::Level(Level::Ultra)).unwrap();
    let custom = graphics.resolved();
    assert!(custom.custom);
    assert_eq!(custom.preset, Level::Low);
    assert_eq!(custom.engine.shadow_distance, 450.0);
    assert_eq!(custom.engine.load_distance, 400.0);
    assert!(custom.generation > low.generation);

    // Setting the preset's own value is no change.
    graphics.set("shadows", SettingValue::Level(Level::Low)).unwrap();
    assert!(!graphics.resolved().custom);

    // Picking a preset again overrides values set by hand.
    graphics.set(defs::VSYNC, SettingValue::Toggle(false)).unwrap();
    graphics.set_preset(Level::Ultra);
    let ultra = graphics.resolved();
    assert!(!ultra.custom);
    assert!(ultra.engine.vsync);
    assert_eq!(ultra.engine.load_distance, 1350.0);
}

#[test]
fn values_are_checked() {
    let mut graphics = Graphics::default();
    graphics.set(defs::RESOLUTION_SCALE, SettingValue::Number(10.0)).unwrap();
    assert_eq!(graphics.resolved().number(defs::RESOLUTION_SCALE), Some(50.0));
    assert!(graphics.set(defs::MSAA, SettingValue::Choice("16x".into())).is_err());
    assert!(graphics.set(defs::MSAA, SettingValue::Toggle(true)).is_err());
    assert!(graphics.set("nope", SettingValue::Toggle(true)).is_err());
}

#[test]
fn a_projects_setting_is_there_like_the_engines() {
    let graphics = Graphics::new(&project_with_toggle(), PlayerSettings::default());
    let resolved = graphics.resolved();
    // The project's preset to start with.
    assert_eq!(resolved.preset, Level::Medium);
    assert_eq!(resolved.toggle("demo.blood"), Some(true));
    let keys: Vec<String> = graphics.definitions().into_iter().map(|d| d.key).collect();
    assert!(keys.contains(&"demo.blood".to_string()));
    // Hidden: not offered, but still at the preset's value.
    assert!(!keys.contains(&defs::FPS_CAP.to_string()));
    assert_eq!(resolved.number(defs::FPS_CAP), Some(0.0));
    // The project's knob table.
    assert_eq!(resolved.engine.shadow_map_size, 1024);
}

#[test]
fn the_player_file_keeps_unknown_keys_and_fixes_bad_values() {
    let text = r#"(
        preset: Some(Low),
        changed: {
            "from.a.newer.build": Toggle(true),
            "display.resolution_scale": Number(500.0),
            "display.msaa": Choice("32x"),
            "shadows": Level(Ultra),
        },
    )"#;
    let player: PlayerSettings = ron::from_str(text).unwrap();
    let graphics = Graphics::new(&ProjectGraphics::default(), player);
    let resolved = graphics.resolved();
    assert_eq!(resolved.preset, Level::Low);
    assert_eq!(resolved.level("shadows"), Level::Ultra);
    assert_eq!(resolved.number(defs::RESOLUTION_SCALE), Some(100.0));
    assert_eq!(resolved.choice(defs::MSAA), Some("off"));
    let changed = &graphics.player().changed;
    assert!(changed.contains_key("from.a.newer.build"));
    assert!(!changed.contains_key(defs::MSAA));

    // Saved and read back.
    let text = ron::ser::to_string_pretty(graphics.player(), ron::ser::PrettyConfig::default()).unwrap();
    assert_eq!(&ron::from_str::<PlayerSettings>(&text).unwrap(), graphics.player());
}

#[test]
fn preview_shows_a_level_without_changing_the_choice() {
    let mut graphics = Graphics::default();
    graphics.set("vegetation", SettingValue::Level(Level::Medium)).unwrap();
    graphics.set_preview(Some(Level::Low));
    let preview = graphics.resolved();
    assert_eq!(preview.level("vegetation"), Level::Low);
    assert_eq!(preview.level("shadows"), Level::Low);
    graphics.set_preview(None);
    let back = graphics.resolved();
    assert_eq!(back.level("vegetation"), Level::Medium);
    assert_eq!(back.level("shadows"), Level::High);
    assert!(back.generation > preview.generation);
}

#[test]
fn the_project_section_is_left_out_of_files_when_empty() {
    assert!(ProjectGraphics::default().is_default());
    let text = ron::ser::to_string(&project_with_toggle()).unwrap();
    let back: ProjectGraphics = ron::from_str(&text).unwrap();
    assert_eq!(back, project_with_toggle());
    // Fields left out of a project file take their defaults.
    let partial: ProjectGraphics = ron::from_str("(default_preset: Low)").unwrap();
    assert_eq!(partial.default_preset, Level::Low);
    assert!(partial.settings.is_empty());
}

#[test]
fn player_file_path_is_in_the_config_folder() {
    if cfg!(windows) {
        return;
    }
    let path = player_file_path("My/Game").unwrap();
    assert!(path.ends_with("ferrite/My_Game/graphics.ron"), "{}", path.display());
}

#[test]
fn ray_traced_lamp_shadows_are_off_at_every_preset_and_switch_on() {
    let mut graphics = Graphics::default();
    for level in [Level::Low, Level::Medium, Level::High, Level::Ultra] {
        graphics.set_preset(level);
        assert!(!graphics.resolved().engine.ray_traced_lights, "{level:?}");
    }
    graphics
        .set(defs::RAY_TRACED_LIGHTS, SettingValue::Toggle(true))
        .unwrap();
    let on = graphics.resolved();
    assert!(on.engine.ray_traced_lights);
    // Independent of the sun's.
    assert!(!on.engine.ray_traced_shadows);
}

#[test]
fn ambient_occlusion_is_a_choice_off_at_low_and_ssao_above() {
    let mut graphics = Graphics::default();
    for (level, mode) in [
        (Level::Low, AoMode::Off),
        (Level::Medium, AoMode::Ssao),
        (Level::High, AoMode::Ssao),
        (Level::Ultra, AoMode::Ssao),
    ] {
        graphics.set_preset(level);
        assert_eq!(graphics.resolved().engine.ambient_occlusion, mode, "{level:?}");
    }
    graphics.set(defs::AMBIENT_OCCLUSION, SettingValue::Choice("RT-AO".into())).unwrap();
    assert_eq!(graphics.resolved().engine.ambient_occlusion, AoMode::RayTraced);
    // Picked by hand at Low, SSAO gets samples.
    graphics.set_preset(Level::Low);
    graphics.set(defs::AMBIENT_OCCLUSION, SettingValue::Choice("SSAO".into())).unwrap();
    assert!(graphics.resolved().engine.ssao_samples > 0);
}

#[test]
fn reflections_are_a_choice_ssr_at_every_preset() {
    let mut graphics = Graphics::default();
    for level in [Level::Low, Level::Medium, Level::High, Level::Ultra] {
        graphics.set_preset(level);
        assert_eq!(graphics.resolved().engine.reflections, ReflectionMode::Ssr, "{level:?}");
    }
    graphics.set(defs::REFLECTIONS, SettingValue::Choice("RT".into())).unwrap();
    assert_eq!(graphics.resolved().engine.reflections, ReflectionMode::RayTraced);
    graphics.set(defs::REFLECTIONS, SettingValue::Choice("Sky".into())).unwrap();
    assert_eq!(graphics.resolved().engine.reflections, ReflectionMode::Sky);
}
