use super::{
    codegen::{self, Variant},
    graph::*,
    standard,
};

#[test]
fn standard_pbr_has_no_problems() {
    let graph = standard::standard_pbr();
    assert_eq!(graph.problems(), vec![]);
}

#[test]
fn standard_pbr_reads_its_maps_as_before() {
    let code = codegen::generate(&standard::standard_pbr(), Variant { cutout: true, lod0: false }).unwrap();
    // The cut-out before the shadow pass' return, the rest after.
    let discard = code.fragment.find("discard").unwrap();
    let shadow = code.fragment.find("isShadowPass").unwrap();
    assert!(discard < shadow, "{}", code.fragment);
    assert!(code.fragment.contains("texture(mp_base_color_map"));
    assert!(code.fragment.contains("materialNormalMap(mp_normal_map, mp_normal_map_set"));
    assert!(code.vertex.contains("materialWind("));
    assert!(code.moves_vertices);
    assert!(code.opacity.as_ref().unwrap().contains("materialOpacity"));
    // Opaque: no discard, no depth fragment code.
    let opaque = codegen::generate(&standard::standard_pbr(), Variant::default()).unwrap();
    assert!(!opaque.fragment.contains("discard"));
    assert!(opaque.opacity.is_none());
}

#[test]
fn impostor_bake_reads_full_detail() {
    let code = codegen::generate(&standard::standard_pbr(), Variant { cutout: true, lod0: true }).unwrap();
    assert!(code.fragment.contains("textureLod(mp_base_color_map"));
}

#[test]
fn widths_are_converted() {
    assert_eq!(codegen::convert("x", 1, 3), "vec3(x)");
    assert_eq!(codegen::convert("x", 4, 3), "(x).xyz");
    assert_eq!(codegen::convert("x", 3, 1), "(x).x");
    assert_eq!(codegen::convert("x", 2, 4), "vec4(x, 0.0, 1.0)");
}

fn tiny() -> (MaterialGraph, NodeId) {
    let mut g = MaterialGraph::empty();
    let out = g.output_node().unwrap().id;
    (g, out)
}

#[test]
fn a_cycle_is_refused() {
    let (mut g, out) = tiny();
    let a = g.add(NodeKind::Math(MathOp::Add));
    let b = g.add(NodeKind::Math(MathOp::Add));
    g.connect(a, 0, b, 0);
    g.connect(b, 0, a, 0);
    g.connect(b, 0, out, output::ROUGHNESS);
    assert!(g.problems().iter().any(|p| p.message.contains("Kreis")));
    assert!(codegen::generate(&g, Variant::default()).is_err());
}

#[test]
fn a_normal_map_in_the_world_offset_is_refused() {
    let (mut g, out) = tiny();
    let n = g.add(NodeKind::NormalMap);
    g.connect(n, 0, out, output::WORLD_OFFSET);
    let problems = g.problems();
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert_eq!(problems[0].node, Some(n));
}

#[test]
fn wind_in_the_colour_is_refused() {
    let (mut g, out) = tiny();
    let w = g.add(NodeKind::WindSway);
    g.connect(w, 0, out, output::BASE_COLOR);
    assert_eq!(g.problems()[0].node, Some(w));
}

#[test]
fn a_texture_must_come_from_a_texture_parameter() {
    let (mut g, out) = tiny();
    g.params.push(ParamDecl { name: "tint".into(), ty: ParamType::Color, default: [1.0; 4], group: String::new(), label: String::new() });
    let p = g.add(NodeKind::Parameter("tint".into()));
    let s = g.add(NodeKind::SampleTexture);
    g.connect(p, 0, s, 0);
    g.connect(s, 1, out, output::BASE_COLOR);
    assert!(g.problems().iter().any(|p| p.node == Some(s)));
}

#[test]
fn unknown_parameters_and_bad_names_are_refused() {
    let (mut g, out) = tiny();
    let p = g.add(NodeKind::Parameter("missing".into()));
    g.connect(p, 0, out, output::ROUGHNESS);
    g.params.push(ParamDecl { name: "bad name".into(), ty: ParamType::Float, default: [0.0; 4], group: String::new(), label: String::new() });
    let problems = g.problems();
    assert!(problems.iter().any(|p| p.node == Some(p_id(&g, "missing"))));
    assert!(problems.iter().any(|p| p.message.contains("bad name")));
}

fn p_id(g: &MaterialGraph, name: &str) -> NodeId {
    g.nodes.iter().find(|n| n.kind == NodeKind::Parameter(name.into())).unwrap().id
}

#[test]
fn every_node_kind_generates_code() {
    // Each kind wired into a channel it can feed, on its own.
    for kind in NodeKind::palette() {
        let (mut g, out) = tiny();
        g.params.push(ParamDecl {
            name: "map".into(),
            ty: ParamType::Texture { color: true },
            default: [0.0; 4],
            group: String::new(),
            label: String::new(),
        });
        let kind = match kind {
            NodeKind::Parameter(_) => NodeKind::Parameter("map".into()),
            other => other,
        };
        if kind == NodeKind::Parameter("map".into()) {
            continue;
        }
        let n = g.add(kind.clone());
        if matches!(kind, NodeKind::SampleTexture | NodeKind::NormalMap) {
            let p = g.add(NodeKind::Parameter("map".into()));
            g.connect(p, 0, n, 0);
        }
        let pin = if kind.vertex_only() || kind == NodeKind::Normal(Space::Object) {
            output::WORLD_OFFSET
        } else {
            output::EMISSIVE
        };
        g.connect(n, 0, out, pin);
        let code = codegen::generate(&g, Variant::default());
        assert!(code.is_ok(), "{kind:?}: {:?}", code.err());
    }
}

#[test]
fn graphs_round_trip_through_ron() {
    let g = standard::standard_pbr();
    let back = MaterialGraph::from_ron(&g.to_ron()).unwrap();
    assert_eq!(back, g);
}

/// Every node kind and Standard PBR, in every pipeline and variant, as
/// OpenGL GLSL (glslangValidator) and rewritten for Vulkan (glslc). Skipped
/// where the tools are missing. Ignored by default (it runs some 1800
/// compiler processes, about two minutes): `cargo test -p ferrite --lib
/// generated_programs_compile -- --ignored`.
#[test]
#[ignore]
fn generated_programs_compile_for_gl_and_vulkan() {
    use super::programs::{sources, Pipeline};
    let have = |tool: &str| std::process::Command::new(tool).arg("--version").output().is_ok();
    if !have("glslc") || !have("glslangValidator") {
        eprintln!("glslc or glslangValidator missing: skipped");
        return;
    }
    let mut graphs = vec![("standard".to_string(), standard::standard_pbr())];
    for (i, kind) in NodeKind::palette().into_iter().enumerate() {
        let (mut g, out) = tiny();
        g.params.push(ParamDecl {
            name: "map".into(),
            ty: ParamType::Texture { color: true },
            default: [0.0; 4],
            group: String::new(),
            label: String::new(),
        });
        if matches!(kind, NodeKind::Parameter(_)) {
            continue;
        }
        let n = g.add(kind.clone());
        if matches!(kind, NodeKind::SampleTexture | NodeKind::NormalMap) {
            let p = g.add(NodeKind::Parameter("map".into()));
            g.connect(p, 0, n, 0);
        }
        let pin = if kind.vertex_only() || kind == NodeKind::Normal(Space::Object) {
            output::WORLD_OFFSET
        } else {
            output::EMISSIVE
        };
        g.connect(n, 0, out, pin);
        // An opacity too, for the cut-out programs.
        let c = g.add(NodeKind::Constant { value: [0.7; 4], width: 1 });
        g.connect(c, 0, out, output::OPACITY);
        graphs.push((format!("kind{i}"), g));
    }
    let dir = std::env::temp_dir().join("ferrite-material-check");
    std::fs::create_dir_all(&dir).unwrap();
    let mut failures = Vec::new();
    let run = |cmd: &mut std::process::Command| {
        let out = cmd.output().unwrap();
        (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
    };
    for (name, graph) in &graphs {
        for variant in [Variant { cutout: false, lod0: false }, Variant { cutout: true, lod0: false }, Variant { cutout: true, lod0: true }] {
            let code = codegen::generate(graph, variant).unwrap();
            for pipeline in [Pipeline::Mesh, Pipeline::MeshDepth, Pipeline::Instanced, Pipeline::InstancedDepth, Pipeline::ImpostorBake] {
                let (vertex, fragment) = sources(&code, pipeline);
                let stem = format!("{name}-{}{}-{pipeline:?}", variant.cutout as u8, variant.lod0 as u8);
                for (stage, source) in [("vert", &vertex), ("frag", &fragment)] {
                    let file = dir.join(format!("{stem}.{stage}"));
                    std::fs::write(&file, source).unwrap();
                    let (ok, log) = run(std::process::Command::new("glslangValidator").arg(&file));
                    if !ok {
                        failures.push(format!("GL {stem}.{stage}: {log}"));
                    }
                }
                match crate::core::gfx::glsl::to_vulkan(&vertex, &fragment) {
                    Ok(program) => {
                        for (stage, source) in [("vert", &program.vertex), ("frag", &program.fragment)] {
                            let file = dir.join(format!("{stem}.vk.{stage}"));
                            std::fs::write(&file, source).unwrap();
                            let (ok, log) = run(std::process::Command::new("glslc")
                                .args(["--target-env=vulkan1.3", "-fauto-map-locations", "-o"])
                                .arg(dir.join(format!("{stem}.{stage}.spv")))
                                .arg(&file));
                            if !ok {
                                failures.push(format!("VK {stem}.{stage}: {log}"));
                            }
                        }
                    }
                    Err(e) => failures.push(format!("VK {stem}: {e}")),
                }
            }
        }
    }
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures[..failures.len().min(6)].join("\n"));
}

/// Give a project's models their material assets, as the editor does when
/// it opens the project: `FERRITE_MIGRATE_PROJECT=<project dir> cargo test
/// -p ferrite --lib migrate_project -- --ignored`.
#[test]
#[ignore]
fn migrate_project() {
    let dir = std::env::var("FERRITE_MIGRATE_PROJECT").expect("FERRITE_MIGRATE_PROJECT");
    let mut database = crate::core::assets::AssetDatabase::new(std::path::Path::new(&dir).join("assets"));
    let problems = database.scan();
    assert!(problems.is_empty(), "{problems:?}");
    let (made, problems) = super::asset::ensure_all_model_materials(&mut database);
    println!("made {made} materials");
    assert!(problems.is_empty(), "{problems:?}");
}

#[test]
fn parameters_have_names_for_people() {
    assert_eq!(display_name("uv_scale"), "UV Scale");
    assert_eq!(display_name("two_sided"), "Two Sided");
    let graph = standard::standard_pbr();
    assert_eq!(graph.param("uv_scale").unwrap().display(), "UV Scale");
    assert_eq!(super::edit::property_label("emissive_strength", &graph), "Glow Strength");
    assert_eq!(super::edit::property_label("two_sided", &graph), "Two-Sided");
    assert_eq!(super::edit::property_label("param:3:default", &graph), "Default");
}
