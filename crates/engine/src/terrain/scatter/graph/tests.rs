//! The rules of docs/scatter-graphs.md ("Rules and how each is checked").

use super::*;
use crate::core::entity::component::PropertyValue;
use crate::terrain::{
    chunk::scatter_points_at,
    hills::{Hills, HillsSettings},
    source::{ChunkCoord, TerrainGenerator, TerrainSource},
};

/// A plane: height `tilt * x`; nothing grows west of `bare_west_of`; the
/// chance is `chance` elsewhere.
struct Plane {
    tilt: f32,
    bare_west_of: f32,
    chance: f32,
}

impl Plane {
    fn flat() -> Self {
        Plane { tilt: 0.0, bare_west_of: f32::MIN, chance: 1.0 }
    }

    fn tilted(tilt: f32) -> Self {
        Plane { tilt, ..Self::flat() }
    }
}

impl Ground for Plane {
    fn height(&self, x: f64, _z: f64) -> f32 {
        self.tilt * x as f32
    }

    fn chance(&self, _tag: &str, x: f64, _y: f32, _z: f64) -> f32 {
        if (x as f32) < self.bare_west_of {
            0.0
        } else {
            self.chance
        }
    }
}

fn grass() -> NodeKind {
    NodeKind::SpawnMesh(SpawnMesh {
        asset: None,
        model: "grass/grass.glb".into(),
        range: ScatterRange::Detail { distance: 60.0 },
        collision: false,
        shadows: false,
        wind: 1.0,
        tint: 0.0,
        normals_up: 0.0,
    })
}

fn surface(spacing: f32) -> NodeKind {
    NodeKind::SurfacePoints { spacing, jitter: 1.0, seed: 11, world_seed: false }
}

/// The placements of the graph's first output.
fn placed<G: Ground + ?Sized>(graph: &ScatterGraph, ground: &G, rect: Rect) -> Vec<Placement> {
    let eval = evaluate(graph, ground, rect).unwrap();
    eval.outputs.into_iter().next().unwrap().1
}

fn spots(placements: &[Placement]) -> Vec<[u32; 3]> {
    let mut spots: Vec<[u32; 3]> = placements.iter().map(|p| p.position.map(f32::to_bits)).collect();
    spots.sort();
    spots
}

/// The four quarters of `rect`.
fn quarters(rect: Rect) -> [Rect; 4] {
    let (mx, mz) = ((rect.x0 + rect.x1) / 2.0, (rect.z0 + rect.z1) / 2.0);
    [
        Rect::new(rect.x0, rect.z0, mx, mz),
        Rect::new(mx, rect.z0, rect.x1, mz),
        Rect::new(rect.x0, mz, mx, rect.z1),
        Rect::new(mx, mz, rect.x1, rect.z1),
    ]
}

#[test]
fn the_same_graph_places_the_same_every_time_and_on_any_thread() {
    let graph = ScatterGraph::chain([
        surface(0.7),
        NodeKind::Noise { wavelength: 12.0, threshold: 0.4, softness: 0.2, seed: 3, world_seed: false },
        NodeKind::DensityCut { multiplier: 1.0 },
        NodeKind::Rotate { yaw_min: 0.0, yaw_max: 360.0, align: 0.5, tilt: 10.0 },
        NodeKind::Scale { min: 0.3, max: 0.6, height_min: 0.8, height_max: 1.4, by_density: 0.5 },
        grass(),
    ]);
    let ground = Plane::tilted(0.2);
    let rect = Rect::new(-20.0, 5.0, 12.0, 37.0);
    let first = placed(&graph, &ground, rect);
    assert!(!first.is_empty());
    assert_eq!(first, placed(&graph, &ground, rect));
    let (a, b) = std::thread::scope(|s| {
        let a = s.spawn(|| placed(&graph, &ground, rect));
        let b = s.spawn(|| placed(&graph, &ground, rect));
        (a.join().unwrap(), b.join().unwrap())
    });
    assert_eq!(first, a);
    assert_eq!(first, b);
}

#[test]
fn a_layer_as_a_graph_keeps_its_spots() {
    let hills = Hills::create(7, HillsSettings::default());
    let salt = 0x5eed_1234;
    let graph = ScatterGraph::random_layer(None, 60.0, "tree", salt);
    let attempts = (60.0f32 * 128.0 * 128.0 / 10_000.0).round() as u32;
    let mut compared = 0;
    for (cx, cz) in [(0, 0), (3, -2), (-1, 5), (-4, -4)] {
        let got: Vec<(u32, u32, u32, u32)> = {
            let mut v: Vec<_> = placed(&graph, &*hills, Rect::column(cx, cz))
                .iter()
                .map(|p| (p.position[0].to_bits(), p.position[1].to_bits(), p.position[2].to_bits(), p.yaw.to_bits()))
                .collect();
            v.sort();
            v
        };
        let mut expected: Vec<(u32, u32, u32, u32)> = hills
            .vertical_range((cx, cz), false)
            .flat_map(|cy| scatter_points_at(&*hills, ChunkCoord::new(cx, cy, cz), "tree", salt, attempts))
            .map(|(x, y, z, yaw)| (x.to_bits(), y.to_bits(), z.to_bits(), yaw.to_bits()))
            .collect();
        expected.sort();
        assert_eq!(got, expected, "column ({cx}, {cz})");
        compared += got.len();
    }
    // Some columns are under the sea; not all of them.
    assert!(compared > 0);
}

#[test]
fn nothing_grows_where_the_world_says_no() {
    let graph = ScatterGraph::chain([
        surface(1.0),
        NodeKind::WorldChance { tag: "grass".into() },
        NodeKind::DensityCut { multiplier: 1.0 },
        grass(),
    ]);
    let ground = Plane { bare_west_of: 16.0, ..Plane::flat() };
    let placements = placed(&graph, &ground, Rect::new(0.0, 0.0, 32.0, 32.0));
    assert!(!placements.is_empty());
    assert!(placements.iter().all(|p| p.position[0] >= 16.0));
}

#[test]
fn a_density_cut_keeps_about_the_chance() {
    let graph = ScatterGraph::chain([surface(0.5), NodeKind::WorldChance { tag: String::new() }, NodeKind::DensityCut { multiplier: 1.0 }, grass()]);
    let ground = Plane { chance: 0.3, ..Plane::flat() };
    let eval = evaluate(&graph, &ground, Rect::new(0.0, 0.0, 40.0, 40.0)).unwrap();
    let all = 80 * 80;
    let kept = eval.outputs[0].1.len();
    assert!((kept as f32 / all as f32 - 0.3).abs() < 0.03, "{kept} of {all}");
    // Every node says how many points it put out.
    assert_eq!(eval.counts[&NodeId(0)], all);
    assert_eq!(eval.counts[&NodeId(2)], kept);
    assert_eq!(eval.counts[&NodeId(3)], kept);
}

#[test]
fn the_slope_filter_keeps_what_it_says() {
    let rect = Rect::new(0.0, 0.0, 20.0, 20.0);
    let gentle = |min, max| ScatterGraph::chain([surface(1.0), NodeKind::Slope { min, max, soft: 0.0 }, grass()]);
    // 45 degrees.
    let steep = Plane::tilted(1.0);
    assert!(placed(&gentle(0.0, 30.0), &steep, rect).is_empty());
    assert_eq!(placed(&gentle(40.0, 50.0), &steep, rect).len(), 400);
    assert_eq!(placed(&gentle(0.0, 5.0), &Plane::flat(), rect).len(), 400);
    // A soft edge lowers the density instead of cutting.
    let soft = ScatterGraph::chain([
        surface(1.0),
        NodeKind::Slope { min: 0.0, max: 40.0, soft: 10.0 },
        NodeKind::DensityCut { multiplier: 1.0 },
        grass(),
    ]);
    let some = placed(&soft, &steep, rect).len();
    assert!(some > 0 && some < 400, "{some}");
}

#[test]
fn the_height_filter_keeps_what_it_says() {
    let graph = ScatterGraph::chain([surface(1.0), NodeKind::Height { min: 10.0, max: 20.0, soft: 0.0 }, grass()]);
    // Height = x.
    let placements = placed(&graph, &Plane::tilted(1.0), Rect::new(0.0, 0.0, 40.0, 10.0));
    assert!(!placements.is_empty());
    assert!(placements.iter().all(|p| (10.0..=20.0).contains(&p.position[1])));
}

/// Grass kept 2 m away from random rocks.
fn grass_around_rocks() -> (ScatterGraph, NodeId) {
    let mut graph = ScatterGraph::default();
    let points = graph.add(surface(0.8));
    let rocks = graph.add(NodeKind::RandomPoints { per_hectare: 3000.0, seed: 5, world_seed: false });
    let away = graph.add(NodeKind::KeepAwayFrom { radius: 2.0 });
    let out = graph.add(grass());
    graph.connect(points, 0, away, 0);
    graph.connect(rocks, 0, away, 1);
    graph.connect(away, 0, out, 0);
    (graph, rocks)
}

#[test]
fn keep_away_respects_the_radius_across_the_edge() {
    let (graph, _) = grass_around_rocks();
    let rect = Rect::new(0.0, 0.0, 16.0, 16.0);
    let blades = placed(&graph, &Plane::flat(), rect);
    assert!(!blades.is_empty());
    // The rocks, also those just past the edge.
    let rocks_only = ScatterGraph::chain([NodeKind::RandomPoints { per_hectare: 3000.0, seed: 5, world_seed: false }, grass()]);
    let rocks = placed(&rocks_only, &Plane::flat(), Rect::new(-3.0, -3.0, 19.0, 19.0));
    assert!(rocks.iter().any(|r| !rect.contains(r.position[0], r.position[2])));
    for g in &blades {
        for r in &rocks {
            let d = (g.position[0] - r.position[0]).hypot(g.position[2] - r.position[2]);
            assert!(d > 2.0, "grass at {:?} is {d} m from a rock", g.position);
        }
    }
}

#[test]
fn cutting_the_ground_into_cells_changes_nothing() {
    let rect = Rect::new(-16.0, -16.0, 16.0, 16.0);
    let (around_rocks, _) = grass_around_rocks();
    let graphs = [
        ScatterGraph::chain([surface(0.6), NodeKind::Noise { wavelength: 8.0, threshold: 0.5, softness: 0.3, seed: 1, world_seed: false }, NodeKind::DensityCut { multiplier: 1.0 }, grass()]),
        ScatterGraph::chain([NodeKind::RandomPoints { per_hectare: 900.0, seed: 2, world_seed: false }, grass()]),
        around_rocks,
    ];
    let ground = Plane::tilted(0.3);
    for graph in &graphs {
        let whole = spots(&placed(graph, &ground, rect));
        let mut cut: Vec<Placement> = Vec::new();
        for quarter in quarters(rect) {
            cut.extend(placed(graph, &ground, quarter));
        }
        assert!(!whole.is_empty());
        assert_eq!(whole, spots(&cut));
    }
}

#[test]
fn transforms_stay_in_their_ranges() {
    let graph = ScatterGraph::chain([
        surface(1.0),
        NodeKind::Rotate { yaw_min: 10.0, yaw_max: 20.0, align: 1.0, tilt: 0.0 },
        NodeKind::Scale { min: 0.5, max: 0.8, height_min: 1.0, height_max: 2.0, by_density: 0.0 },
        NodeKind::Offset { min: -0.3, max: -0.1 },
        grass(),
    ]);
    // 45 degrees, rising to +x: the normal leans to -x.
    let placements = placed(&graph, &Plane::tilted(1.0), Rect::new(0.0, 0.0, 10.0, 10.0));
    assert_eq!(placements.len(), 100);
    let half = std::f32::consts::FRAC_1_SQRT_2;
    for p in &placements {
        assert!((10f32.to_radians()..=20f32.to_radians()).contains(&p.yaw));
        assert!((p.up[0] + half).abs() < 1e-3 && (p.up[1] - half).abs() < 1e-3, "{:?}", p.up);
        let s = p.scale[0];
        assert!((0.5..=0.8).contains(&s) && p.scale[2] == s);
        assert!((s..=2.0 * s).contains(&p.scale[1]));
        let below = p.position[0] - p.position[1];
        assert!((0.1..=0.3).contains(&below), "{below}");
    }
    // The model's up axis ends up along `up`.
    let up = placements[0].rotation() * cgmath::Vector3::unit_y();
    assert!((up.x + half).abs() < 1e-3 && (up.y - half).abs() < 1e-3);
}

#[test]
fn split_and_merge_keep_every_point() {
    let mut graph = ScatterGraph::default();
    let points = graph.add(surface(1.0));
    let split = graph.add(NodeKind::Split { by: SplitBy::Random, threshold: 0.3 });
    let flowers = graph.add(grass());
    let merge = graph.add(NodeKind::Merge);
    let all = graph.add(grass());
    graph.connect(points, 0, split, 0);
    graph.connect(split, 1, flowers, 0);
    graph.connect(split, 0, merge, 0);
    graph.connect(split, 1, merge, 0);
    graph.connect(merge, 0, all, 0);
    let eval = evaluate(&graph, &Plane::flat(), Rect::new(0.0, 0.0, 40.0, 40.0)).unwrap();
    let below = eval.placements(flowers).len();
    assert!((below as f32 / 1600.0 - 0.3).abs() < 0.04, "{below}");
    assert_eq!(eval.placements(all).len(), 1600);
}

#[test]
fn broken_graphs_are_refused() {
    let mut cycle = ScatterGraph::chain([surface(1.0), NodeKind::Merge, NodeKind::DensityCut { multiplier: 1.0 }, grass()]);
    cycle.connect(NodeId(2), 0, NodeId(1), 0);
    assert!(matches!(cycle.problems()[..], [GraphProblem::Cycle(_)]));
    assert!(evaluate(&cycle, &Plane::flat(), Rect::new(0.0, 0.0, 1.0, 1.0)).is_err());

    let mut twice = ScatterGraph::chain([surface(1.0), NodeKind::DensityCut { multiplier: 1.0 }, grass()]);
    twice.connect(NodeId(0), 0, NodeId(1), 0);
    assert_eq!(twice.problems(), vec![GraphProblem::InputWiredTwice(PinRef { node: NodeId(1), pin: 0 })]);

    let mut nowhere = ScatterGraph::chain([surface(1.0), grass()]);
    nowhere.connect(NodeId(0), 0, NodeId(1), 3);
    nowhere.connect(NodeId(7), 0, NodeId(1), 0);
    assert_eq!(nowhere.problems().len(), 2);

    // Far too fine for the ground asked for.
    let fine = ScatterGraph::chain([surface(0.01), grass()]);
    assert!(evaluate(&fine, &Plane::flat(), Rect::new(0.0, 0.0, 128.0, 128.0)).is_err());
}

#[test]
fn an_unwired_output_gets_nothing() {
    let graph = ScatterGraph::chain([grass()]);
    let eval = evaluate(&graph, &Plane::flat(), Rect::new(0.0, 0.0, 10.0, 10.0)).unwrap();
    assert_eq!(eval.outputs, vec![(NodeId(0), Vec::new())]);
}

#[test]
fn a_graph_reads_back_as_written() {
    let (graph, _) = grass_around_rocks();
    let text = graph.to_ron();
    assert_eq!(ScatterGraph::from_ron(&text).unwrap(), graph);
    // Ids are plain numbers, easy to read and write by hand.
    assert!(text.contains("id: 0,"), "{text}");
    assert!(text.contains("node: 0,"), "{text}");
    assert!(is_scatter_graph_file(Path::new("assets/scatter/Meadow.scatter.ron")));
    assert_eq!(scatter_graph_name(Path::new("assets/scatter/Meadow.scatter.ron")), "Meadow");
}

/// A flat ground of one colour that counts how often it was asked for it.
struct Painted {
    color: [f32; 3],
    asked: std::sync::atomic::AtomicUsize,
}

impl Ground for Painted {
    fn height(&self, _x: f64, _z: f64) -> f32 {
        0.0
    }

    fn chance(&self, _tag: &str, _x: f64, _y: f32, _z: f64) -> f32 {
        1.0
    }

    fn tint(&self, _x: f64, _z: f64) -> [f32; 3] {
        self.asked.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.color
    }
}

fn tinted(amount: f32) -> NodeKind {
    match grass() {
        NodeKind::SpawnMesh(spawn) => NodeKind::SpawnMesh(SpawnMesh { tint: amount, ..spawn }),
        _ => unreachable!(),
    }
}

#[test]
fn models_take_on_as_much_of_the_ground_colour_as_asked() {
    let rect = Rect::new(0.0, 0.0, 5.0, 5.0);
    let ground = Painted { color: [0.5, 1.0, 1.5], asked: Default::default() };
    let none = placed(&ScatterGraph::chain([surface(1.0), tinted(0.0)]), &ground, rect);
    assert!(none.iter().all(|p| p.tint == [1.0; 3]));
    // Not even asked.
    assert_eq!(ground.asked.load(std::sync::atomic::Ordering::Relaxed), 0);
    let full = placed(&ScatterGraph::chain([surface(1.0), tinted(1.0)]), &ground, rect);
    assert!(full.iter().all(|p| p.tint == [0.5, 1.0, 1.5]));
    assert_eq!(ground.asked.load(std::sync::atomic::Ordering::Relaxed), full.len());
    let half = placed(&ScatterGraph::chain([surface(1.0), tinted(0.5)]), &ground, rect);
    assert!(half.iter().all(|p| p.tint == [0.75, 1.0, 1.25]));
}

#[test]
fn the_tint_rides_in_the_bottom_row_of_the_transform() {
    let mut placement = Placement {
        position: [3.0, 4.0, 5.0],
        yaw: 0.7,
        up: [0.2, 0.9, 0.1],
        scale: [0.5, 0.8, 0.5],
        tint: [1.0; 3],
    };
    let plain = placement.matrix();
    // Untinted: a plain transform.
    assert_eq!([plain.x.w, plain.y.w, plain.z.w, plain.w.w], [0.0, 0.0, 0.0, 1.0]);
    placement.tint = [0.6, 1.1, 0.9];
    let mut m = placement.matrix();
    // As the vertex shader reads it.
    let tint = [1.0 + m.x.w, 1.0 + m.y.w, 1.0 + m.z.w];
    assert!(tint.iter().zip([0.6, 1.1, 0.9]).all(|(a, b)| (a - b).abs() < 1e-6), "{tint:?}");
    (m.x.w, m.y.w, m.z.w) = (0.0, 0.0, 0.0);
    assert_eq!(m, plain);
}

#[test]
fn wiring_keeps_the_graph_sound() {
    let mut graph = ScatterGraph::default();
    let a = graph.add_node(surface(1.0), [0.0, 0.0]);
    let b = graph.add_node(surface(2.0), [0.0, 100.0]);
    let cut = graph.add_node(NodeKind::DensityCut { multiplier: 1.0 }, [200.0, 0.0]);
    let merge = graph.add_node(NodeKind::Merge, [400.0, 0.0]);
    let out = graph.add_node(grass(), [600.0, 0.0]);
    let pin = |node, pin| PinRef { node, pin };
    graph.wire(pin(a, 0), pin(cut, 0)).unwrap();
    // A single input takes the new wire in place of the old.
    graph.wire(pin(b, 0), pin(cut, 0)).unwrap();
    assert_eq!(graph.wires_into(cut, 0).map(|w| w.from.node).collect::<Vec<_>>(), vec![b]);
    // A multi input takes both, but the same wire once.
    graph.wire(pin(cut, 0), pin(merge, 0)).unwrap();
    graph.wire(pin(a, 0), pin(merge, 0)).unwrap();
    graph.wire(pin(a, 0), pin(merge, 0)).unwrap();
    assert_eq!(graph.wires_into(merge, 0).count(), 2);
    graph.wire(pin(merge, 0), pin(out, 0)).unwrap();
    // No circles, no pins that are not there, no outputs of an output.
    assert!(graph.wire(pin(merge, 0), pin(cut, 0)).is_err());
    assert!(graph.wire(pin(cut, 0), pin(cut, 0)).is_err());
    assert!(graph.wire(pin(a, 1), pin(cut, 0)).is_err());
    assert!(graph.wire(pin(out, 0), pin(cut, 0)).is_err());
    assert!(graph.problems().is_empty(), "{:?}", graph.problems());
    // Removing a node takes its wires along.
    graph.remove_nodes(&[merge]);
    assert!(graph.wires.iter().all(|w| w.from.node != merge && w.to.node != merge));
    assert!(graph.problems().is_empty());
}

#[test]
fn copied_nodes_paste_with_new_ids_and_their_wires() {
    let mut graph = ScatterGraph::chain([surface(1.0), NodeKind::DensityCut { multiplier: 1.0 }, grass()]);
    let part = graph.copy_nodes(&[NodeId(0), NodeId(1)]);
    assert_eq!(part.wires.len(), 1);
    let pasted = graph.paste(&part, [0.0, 300.0]);
    assert_eq!(pasted, vec![NodeId(3), NodeId(4)]);
    assert!(graph.wires.contains(&Wire { from: PinRef { node: NodeId(3), pin: 0 }, to: PinRef { node: NodeId(4), pin: 0 } }));
    assert_eq!(graph.node(NodeId(3)).unwrap().position, [0.0, 300.0]);
    assert!(graph.problems().is_empty());
}

#[test]
fn every_property_shown_can_be_set() {
    for mut kind in NodeKind::catalog() {
        for property in kind.properties() {
            if property.read_only {
                continue;
            }
            let value = match &property.value {
                PropertyValue::Float(v) => PropertyValue::Float(v + 0.25),
                PropertyValue::Int(v) => PropertyValue::Int(v + 1),
                PropertyValue::Bool(v) => PropertyValue::Bool(!v),
                PropertyValue::Text(_) => match &property.options {
                    Some(options) => PropertyValue::Text(options.last().unwrap().clone()),
                    None => PropertyValue::Text("meadow".into()),
                },
            };
            assert!(kind.set_property(&property.name, &value), "{} / {}", kind.label(), property.name);
            let now = kind.properties().into_iter().find(|p| p.name == property.name);
            if let Some(now) = now {
                assert_eq!(format!("{:?}", now.value), format!("{value:?}"), "{} / {}", kind.label(), property.name);
            }
        }
        assert!(!kind.set_property("no such thing", &PropertyValue::Float(1.0)));
    }
}

#[test]
fn edits_undo_and_redo() {
    let mut graph = ScatterGraph::chain([surface(1.0), grass()]);
    let mut history = GraphHistory::default();
    let original = graph.clone();
    history.record(&graph);
    graph.add_node(NodeKind::DensityCut { multiplier: 1.0 }, [10.0, 10.0]);
    let added = graph.clone();
    history.record(&graph);
    graph.move_nodes(&[NodeId(0)], [5.0, 0.0]);
    assert!(history.undo(&mut graph));
    assert_eq!(graph, added);
    assert!(history.undo(&mut graph));
    assert_eq!(graph, original);
    assert!(!history.undo(&mut graph));
    assert!(history.redo(&mut graph));
    assert_eq!(graph, added);
    // A new edit drops what could be redone.
    history.record(&graph);
    graph.remove_nodes(&[NodeId(2)]);
    assert!(!history.can_redo());
}

#[test]
fn kinds_are_named_by_role_and_found_with_or_without_it() {
    for kind in NodeKind::catalog() {
        let label = kind.label();
        let (role, name) = label.split_once(": ").expect("a role");
        assert!(["Source", "Filter", "Transform", "Flow", "Output"].contains(&role), "{label}");
        assert!(kind.is_named(label) && kind.is_named(name) && kind.is_named(&name.to_uppercase().replace(' ', "_")));
        // Only itself.
        assert_eq!(NodeKind::catalog().iter().filter(|k| k.is_named(name)).count(), 1, "{label}");
    }
    assert_eq!(NodeKind::Slope { min: 0.0, max: 1.0, soft: 0.0 }.role(), NodeRole::Filter);
}

#[test]
fn every_kind_explains_itself_and_names_only_settings_it_has() {
    for kind in NodeKind::catalog() {
        let text = kind.description();
        assert!(text.len() > 30, "{}", kind.label());
        // Settings in backticks are the node's own.
        let names: Vec<String> = kind.properties().iter().map(|p| p.name.to_string()).collect();
        for quoted in text.split('`').skip(1).step_by(2) {
            assert!(names.iter().any(|n| n == quoted), "{}: `{quoted}`", kind.label());
        }
    }
}

#[test]
fn a_traced_run_keeps_every_nodes_points() {
    let graph = ScatterGraph::chain([
        surface(1.0),
        NodeKind::WorldChance { tag: String::new() },
        NodeKind::DensityCut { multiplier: 1.0 },
        grass(),
    ]);
    let ground = Plane { chance: 0.5, ..Plane::flat() };
    let rect = Rect::new(0.0, 0.0, 20.0, 20.0);
    let traced = evaluate_traced(&graph, &ground, rect).unwrap();
    let plain = evaluate(&graph, &ground, rect).unwrap();
    assert!(plain.trace.is_empty());
    assert_eq!(traced.outputs, plain.outputs);
    for (id, count) in &traced.counts {
        let points: usize = traced.trace[id].iter().map(Vec::len).sum();
        assert_eq!(points, *count, "node {}", id.0);
    }
    // What the cut kept is among what came in.
    let before: Vec<_> = traced.trace[&NodeId(1)][0].clone();
    // The world chance halved every point's density.
    assert!(before.iter().all(|p| (p[3] - 0.5).abs() < 1e-6));
    assert!(traced.trace[&NodeId(2)][0].iter().all(|p| before.contains(p)));
    assert!(traced.trace[&NodeId(2)][0].len() < before.len());
}

#[test]
fn the_density_cut_multiplier_keeps_more_or_fewer() {
    let ground = Plane { chance: 0.3, ..Plane::flat() };
    let rect = Rect::new(0.0, 0.0, 40.0, 40.0);
    let kept = |multiplier: f32| {
        let graph = ScatterGraph::chain([
            surface(0.5),
            NodeKind::WorldChance { tag: String::new() },
            NodeKind::DensityCut { multiplier },
            grass(),
        ]);
        placed(&graph, &ground, rect).len() as f32 / (80.0 * 80.0)
    };
    assert!((kept(1.0) - 0.3).abs() < 0.03);
    assert!((kept(2.0) - 0.6).abs() < 0.03, "{}", kept(2.0));
    assert!((kept(0.5) - 0.15).abs() < 0.03, "{}", kept(0.5));
    // Beyond certain, all of them; nothing at 0.
    assert_eq!(kept(4.0), 1.0);
    assert_eq!(kept(0.0), 0.0);
}

#[test]
fn a_graph_from_before_the_multiplier_still_loads() {
    let old = "(nodes: [(id: 0, kind: SurfacePoints(spacing: 1.0, jitter: 1.0, seed: 1)), (id: 1, kind: DensityCut), \
               (id: 2, kind: DensityCut , position: (1.0, 2.0))], wires: [])";
    let graph = ScatterGraph::from_ron(old).unwrap();
    assert_eq!(graph.nodes[1].kind, NodeKind::DensityCut { multiplier: 1.0 });
    assert_eq!(graph.nodes[2].kind, NodeKind::DensityCut { multiplier: 1.0 });
    assert_eq!(graph.nodes[2].position, [1.0, 2.0]);
    // A new one reads back as written.
    let new = ScatterGraph::chain([NodeKind::DensityCut { multiplier: 2.5 }]);
    assert_eq!(ScatterGraph::from_ron(&new.to_ron()).unwrap(), new);
}

#[test]
fn a_traced_run_counts_nodes_that_lead_to_no_output_yet() {
    // A chain being built: nothing planted from it yet.
    let mut graph = ScatterGraph::chain([surface(1.0), NodeKind::WorldChance { tag: String::new() }]);
    let out = graph.add(grass());
    let ground = Plane { chance: 0.5, ..Plane::flat() };
    let rect = Rect::new(0.0, 0.0, 10.0, 10.0);
    let traced = evaluate_traced(&graph, &ground, rect).unwrap();
    assert_eq!(traced.counts[&NodeId(0)], 100);
    assert_eq!(traced.counts[&NodeId(1)], 100);
    assert_eq!(traced.trace[&NodeId(1)][0].len(), 100);
    assert_eq!(traced.counts[&out], 0);
    // Planting works out only what grows.
    let planted = evaluate(&graph, &ground, rect).unwrap();
    assert!(!planted.counts.contains_key(&NodeId(0)));
}

#[test]
fn nodes_taking_the_world_seed_change_with_the_world_and_keep_apart() {
    let seeded = |n: u64| NodeKind::SurfacePoints { spacing: 1.0, jitter: 1.0, seed: n, world_seed: true };
    let mut graph = ScatterGraph::default();
    let a = graph.add(seeded(1));
    let b = graph.add(seeded(2));
    let fixed = graph.add(NodeKind::SurfacePoints { spacing: 1.0, jitter: 1.0, seed: 3, world_seed: false });
    let seed_of = |g: &ScatterGraph, id: NodeId| match g.node(id).unwrap().kind {
        NodeKind::SurfacePoints { seed, world_seed, .. } => (seed, world_seed),
        _ => unreachable!(),
    };
    let one = graph.with_world_seed(10);
    let two = graph.with_world_seed(11);
    // Resolved: no longer marked.
    assert!(!seed_of(&one, a).1);
    assert_ne!(seed_of(&one, a).0, seed_of(&two, a).0);
    assert_ne!(seed_of(&one, a).0, seed_of(&one, b).0);
    assert_eq!(seed_of(&one, fixed), (3, false));
    assert_eq!(one, graph.with_world_seed(10));
    // And so the points.
    let rect = Rect::new(0.0, 0.0, 8.0, 8.0);
    let chain = |seed: bool| ScatterGraph::chain([NodeKind::SurfacePoints { spacing: 1.0, jitter: 1.0, seed: 1, world_seed: seed }, grass()]);
    assert_ne!(
        spots(&placed(&chain(true).with_world_seed(10), &Plane::flat(), rect)),
        spots(&placed(&chain(true).with_world_seed(11), &Plane::flat(), rect))
    );
    assert_eq!(
        spots(&placed(&chain(false).with_world_seed(10), &Plane::flat(), rect)),
        spots(&placed(&chain(false).with_world_seed(11), &Plane::flat(), rect))
    );
}

/// Half the ground is "Forest" (x < 10), fading over 2 m; the rest "Plains".
struct Biomes;

impl Ground for Biomes {
    fn height(&self, _x: f64, _z: f64) -> f32 {
        0.0
    }

    fn chance(&self, _tag: &str, _x: f64, _y: f32, _z: f64) -> f32 {
        1.0
    }

    fn biome_weight(&self, biome: &str, x: f64, _z: f64) -> f32 {
        let forest = ((11.0 - x) / 2.0).clamp(0.0, 1.0) as f32;
        match biome {
            "Forest" => forest,
            "Plains" => 1.0 - forest,
            _ => 0.0,
        }
    }
}

#[test]
fn the_biome_filter_keeps_its_biome_and_fades_at_the_border() {
    let rect = Rect::new(0.0, 0.0, 20.0, 20.0);
    let forest = |cut: bool| {
        let mut kinds = vec![surface(0.5), NodeKind::Biome { biome: "Forest".into() }];
        if cut {
            kinds.push(NodeKind::DensityCut { multiplier: 1.0 });
        }
        kinds.push(grass());
        placed(&ScatterGraph::chain(kinds), &Biomes, rect)
    };
    let all = forest(false);
    assert!(all.iter().all(|p| p.position[0] < 11.0));
    assert!(all.iter().any(|p| p.position[0] < 9.0));
    // In the fading band, the cut thins them.
    let band = |ps: &[Placement]| ps.iter().filter(|p| (9.0..11.0).contains(&p.position[0])).count();
    assert!(band(&forest(true)) < band(&all));
    // An unknown biome: nothing.
    let none = ScatterGraph::chain([surface(1.0), NodeKind::Biome { biome: "Desert".into() }, grass()]);
    assert!(placed(&none, &Biomes, rect).is_empty());
}

/// A wall along x = 10 ("houses"); nothing else in reach.
struct Wall;

impl Ground for Wall {
    fn height(&self, _x: f64, _z: f64) -> f32 {
        0.0
    }

    fn chance(&self, _tag: &str, _x: f64, _y: f32, _z: f64) -> f32 {
        1.0
    }

    fn distance(&self, feature: &str, x: f64, _z: f64) -> f32 {
        match feature {
            "houses" => (x as f32 - 10.0).abs(),
            _ => f32::INFINITY,
        }
    }
}

#[test]
fn a_clearance_keeps_the_ground_bare_around_its_feature() {
    let rect = Rect::new(0.0, 0.0, 20.0, 20.0);
    let clear = |feature: &str, distance: f32, soft: f32, cut: bool| {
        let mut kinds = vec![surface(0.5), NodeKind::Clearance { feature: feature.into(), distance, soft }];
        if cut {
            kinds.push(NodeKind::DensityCut { multiplier: 1.0 });
        }
        kinds.push(grass());
        placed(&ScatterGraph::chain(kinds), &Wall, rect)
    };
    let hard = clear("houses", 3.0, 0.0, false);
    assert!(!hard.is_empty());
    assert!(hard.iter().all(|p| (p.position[0] - 10.0).abs() >= 3.0));
    // Soft: still bare within 3 m, thinned out to 5 m, full beyond.
    let soft = clear("houses", 3.0, 2.0, true);
    let near = |ps: &[Placement], a: f32, b: f32| ps.iter().filter(|p| (a..b).contains(&(p.position[0] - 10.0).abs())).count();
    assert_eq!(near(&soft, 0.0, 3.0), 0);
    assert!(near(&soft, 3.0, 5.0) < near(&hard, 3.0, 5.0));
    assert_eq!(near(&soft, 5.5, 9.0), near(&hard, 5.5, 9.0));
    // A feature the world has none of within reach keeps everything.
    assert_eq!(clear("power lines", 50.0, 0.0, false).len(), 1600);
}
