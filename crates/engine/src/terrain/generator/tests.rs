use std::{sync::Arc, thread, time::Instant};

use super::*;
use super::roads::{TUNNEL_HALF_WIDTH, TUNNEL_WALL};

#[test]
fn a_column_holds_the_surface_height_at_every_metre() {
    let generator = WorldGenerator::new(2);
    for (cx, cz) in [(0, 0), (-3, 2), (40, -17)] {
        let column = generator.column(cx, cz);
        for (x, z) in [(-9, -9), (0, 0), (17, 100), (137, 137), (64, -5)] {
            let h = generator.surface_height(cx as f64 * 128.0 + x as f64, cz as f64 * 128.0 + z as f64);
            assert_eq!(column.height(x, z).to_bits(), h.to_bits(), "({cx}, {cz}): ({x}, {z})");
        }
    }
}

#[test]
fn the_ground_has_no_steps_where_tiles_meet() {
    let generator = WorldGenerator::new(5);
    // Across every tile border on a long line, the ground neither jumps
    // nor bends.
    let h = |x: f64| generator.surface_height(x, 1234.5) as f64;
    for i in -16..16 {
        let x = i as f64 * 128.0;
        let jump = h(x + 1e-3) - h(x - 1e-3);
        assert!(jump.abs() < 0.02, "jump of {jump} m at x = {x}");
        let (left, right) = ((h(x) - h(x - 0.05)) / 0.05, (h(x + 0.05) - h(x)) / 0.05);
        assert!((right - left).abs() < 0.1, "slope {left} | {right} at x = {x}");
    }
}

#[test]
fn the_spawn_is_on_low_land_and_the_world_ends_in_deep_sea() {
    for seed in [1, 2, 3, 99] {
        let generator = WorldGenerator::new(seed);
        for (x, z) in [(0.0, 0.0), (150.0, -200.0), (-300.0, 50.0)] {
            let h = generator.surface_height(x, z);
            assert!(h > SEA_LEVEL + 3.0 && h < 50.0, "seed {seed}: {h} m at ({x}, {z})");
        }
        let edge = generator.settings().size / 2.0;
        for (x, z) in [(edge, 0.0), (0.0, -edge - 500.0), (edge + 2000.0, edge)] {
            let h = generator.surface_height(x, z);
            assert!(h < -150.0, "seed {seed}: {h} m at ({x}, {z})");
        }
    }
}

#[test]
fn the_world_has_sea_plains_and_high_mountains() {
    let generator = WorldGenerator::new(2);
    let half = generator.settings().size / 2.0;
    let (mut sea, mut low, mut high, mut highest, mut deepest) = (0, 0, 0, f32::MIN, f32::MAX);
    let n = 120;
    for j in 0..n {
        for i in 0..n {
            let x = -half + (i as f64 + 0.5) * 2.0 * half / n as f64;
            let z = -half + (j as f64 + 0.5) * 2.0 * half / n as f64;
            // The large shape alone: making the fine one for so many
            // far-apart points would take long.
            let h = generator.coarse_height(x, z);
            highest = highest.max(h);
            deepest = deepest.min(h);
            if h < SEA_LEVEL {
                sea += 1;
            } else if h < 150.0 {
                low += 1;
            } else if h > 600.0 {
                high += 1;
            }
        }
    }
    let total = (n * n) as f32;
    println!("sea {:.0}%, low land {:.0}%, above 600 m {:.0}%, {deepest:.0}..{highest:.0} m",
        sea as f32 / total * 100.0, low as f32 / total * 100.0, high as f32 / total * 100.0);
    assert!(sea as f32 / total > 0.2 && (sea as f32 / total) < 0.75);
    assert!(low as f32 / total > 0.1);
    assert!(high > 0 && highest > 900.0 && highest < 2100.0);
    assert!(deepest > -450.0 && deepest < -300.0);
}

#[test]
fn a_column_comes_out_the_same_on_any_thread_and_from_the_cache() {
    let reference = WorldGenerator::new(7).make_column(5, -4);
    let generator = WorldGenerator::new(7);
    // Fill the cache past its size from several threads, so the column
    // is made, evicted and made again.
    let handles: Vec<_> = (0..4)
        .map(|t| {
            let generator = Arc::clone(&generator);
            thread::spawn(move || {
                for i in 0..(COLUMN_CACHE as i32 / 2) {
                    generator.column(i, t);
                }
                generator.column(5, -4)
            })
        })
        .collect();
    for handle in handles {
        let column = handle.join().unwrap();
        assert!(column
            .heights
            .iter()
            .zip(&reference.heights)
            .all(|(a, b)| a.to_bits() == b.to_bits()));
    }
    assert!(generator.columns.lock().unwrap().len() <= COLUMN_CACHE);
}

#[test]
fn a_column_knows_its_lowest_and_highest_ground() {
    let generator = WorldGenerator::new(3);
    let column = generator.column(0, 0);
    let span = Column::SPAN - COLUMN_MARGIN;
    let heights: Vec<f32> = (-COLUMN_MARGIN..span)
        .flat_map(|z| (-COLUMN_MARGIN..span).map(move |x| (x, z)))
        .map(|(x, z)| column.height(x, z))
        .collect();
    assert_eq!(column.min_height, heights.iter().cloned().fold(f32::MAX, f32::min));
    assert_eq!(column.max_height, heights.iter().cloned().fold(f32::MIN, f32::max));
}

#[test]
fn chunk_coords_round_down_to_the_chunk_that_holds_a_point() {
    assert_eq!(ChunkCoord::containing(0.0, 0.0, 0.0), ChunkCoord::new(0, 0, 0));
    assert_eq!(ChunkCoord::containing(-0.5, 127.9, 128.0), ChunkCoord::new(-1, 0, 1));
    assert_eq!(ChunkCoord::new(-2, 1, 3).origin(), [-256.0, 128.0, 384.0]);
    let c = ChunkCoord::new(-4, 0, 9);
    assert_eq!(ChunkCoord::from_position(c.as_position()), c);
}

/// How long the parts of a chunk take. Not a check; run it with
/// `cargo test -p ferrite --release generation_timings -- --ignored --nocapture`.
#[test]
#[ignore]
fn generation_timings() {
    use crate::terrain::{dual_contouring::DualContouringChunk, Chunk};
    let generator = WorldGenerator::new(2);
    generator.roads();
    let n = 16;
    let start = Instant::now();
    for i in 0..n {
        generator.make_column(i, 1000);
    }
    let column = start.elapsed() / n as u32;
    // Inland, by rivers (seed 1).
    let inland = WorldGenerator::new(1);
    inland.roads();
    let start = Instant::now();
    for i in 0..n {
        inland.make_column(20 + i, 14);
    }
    println!("inland column: {:?}", start.elapsed() / n as u32);
    // Along a road.
    let way = &inland.roads().unwrap().ways()[0];
    let start = Instant::now();
    for k in 0..n as usize {
        let ([x, z], _) = way[k * way.len() / n as usize];
        let c = ChunkCoord::containing(x, 0.0, z);
        let near = inland.near(c.x, c.z).works.map_or(0, |w| w.1.len());
        inland.make_column(c.x, c.z);
        if k == 0 {
            println!("{near} road pieces near");
        }
    }
    println!("road column: {:?}", start.elapsed() / n as u32);
    let start = Instant::now();
    for i in 0..n {
        DualContouringChunk::generate(&generator, (i as f32, 0.0, 2000.0), 0);
    }
    let chunk = start.elapsed() / n as u32;
    println!("column: {column:?}, whole chunk (column, 4 meshes, water): {chunk:?}");
}

#[test]
fn caves_open_under_the_mountains_and_the_lattice_follows_them() {
    let generator = WorldGenerator::new(1);
    // A mountain column with a cave 40 m under its surface: there is one
    // in these mountains.
    let (x, z, surface) = (0..200 * 200)
        .map(|k| (-2345.0 + (k % 200 - 100) as f64 * 37.0, -5892.0 + (k / 200 - 100) as f64 * 37.0))
        .map(|(x, z)| (x, z, generator.surface_height(x, z) as f64))
        .find(|&(x, z, surface)| surface > 150.0 && generator.density(x, surface - 40.0, z) > 0.0)
        .expect("no cave");
    let coord = ChunkCoord::containing(x, surface - 40.0, z);
    let lattice = generator.underground(coord, true).expect("caves in the chunk");
    let column = generator.column(coord.x, coord.z);
    let [x0, y0, z0] = coord.origin();
    // Where the exact field is clearly in rock or air, the blended one
    // agrees nearly everywhere.
    let (mut agree, mut total) = (0, 0);
    for k in 0..4000 {
        let (lx, ly, lz) = ((k * 37) % 128, (k * 101) % 128, (k * 59) % 128);
        let (wx, wy, wz) = (x0 + lx as f64, y0 + ly as f64, z0 + lz as f64);
        let exact = generator.density(wx, wy, wz);
        if exact.abs() < 1.5 {
            continue;
        }
        let [push, cave] = lattice.at(lx, ly, lz);
        let blended = (wy as f32 - column.height(lx, lz) - push).max(cave);
        total += 1;
        agree += (exact.signum() == blended.signum()) as i32;
    }
    assert!(agree as f32 > 0.98 * total as f32, "{agree} of {total}");
}

#[test]
fn no_cave_opens_near_the_spawn() {
    let generator = WorldGenerator::new(1);
    for i in 0..400 {
        let (x, z) = ((i % 20) as f64 * 50.0 - 500.0, (i / 20) as f64 * 50.0 - 500.0);
        let h = generator.surface_height(x, z) as f64;
        for depth in [0.5, 2.0, 5.0] {
            assert!(generator.density(x, h - depth, z) < 0.0, "hole at ({x}, {z}), {depth} m down");
        }
    }
}

#[test]
fn without_caves_and_overhangs_the_ground_is_the_height_field() {
    let settings = WorldSettings { caves: 0.0, overhangs: 0.0, ..Default::default() };
    let generator = WorldGenerator::with_settings(1, settings);
    assert!(generator.underground(ChunkCoord::new(-19, 1, -46), true).is_none());
    let h = generator.surface_height(-2345.0, -5892.0) as f64;
    assert_eq!(generator.density(-2345.0, h - 40.0, -5892.0), -40.0);
}

/// Time to make every chunk of a few mountain columns, with and without
/// caves and overhangs: `cargo test --release -p ferrite underground_timings -- --ignored --nocapture`.
#[test]
#[ignore]
fn underground_timings() {
    use crate::terrain::{dual_contouring::DualContouringChunk, Chunk};
    for (caves, overhangs) in [(0.0, 0.0), (1.0, 1.0)] {
        let generator = WorldGenerator::with_settings(1, WorldSettings { caves, overhangs, ..Default::default() });
        let (mut chunks, mut lattice) = (0, std::time::Duration::ZERO);
        let start = Instant::now();
        for cx in -18..-14 {
            let cz = -46;
            for cy in DualContouringChunk::vertical_range(&generator, (cx, cz), true) {
                let t = Instant::now();
                let _ = generator.underground(ChunkCoord::new(cx, cy, cz), true);
                lattice += t.elapsed();
                DualContouringChunk::generate(&generator, (cx as f32, cy as f32, cz as f32), 0);
                chunks += 1;
            }
        }
        println!(
            "caves {caves}: {chunks} chunks, {:?} per chunk, of which lattice {:?}",
            start.elapsed() / chunks,
            lattice / chunks
        );
    }
}

/// A river piece of seed 1 well inside the land, wide enough to see.
fn a_river(generator: &WorldGenerator) -> hydrology::Segment {
    generator
        .hydrology()
        .unwrap()
        .pieces()
        .into_iter()
        .filter(|s| s.width[0] > 8.0 && s.level[1] > 20.0)
        .min_by(|a, b| a.a[0].hypot(a.a[1]).total_cmp(&b.a[0].hypot(b.a[1])))
        .expect("seed 1 has rivers")
}

#[test]
fn rivers_run_downhill_into_lakes_or_the_sea() {
    let generator = WorldGenerator::new(1);
    let plan = generator.hydrology().unwrap();
    let pieces = plan.pieces();
    let (_, lakes) = plan.counts();
    assert!(pieces.len() > 200, "{} river pieces", pieces.len());
    assert!(lakes > 100, "{lakes} lake cells");
    for s in &pieces {
        assert!(s.level[1] <= s.level[0], "{s:?}");
        assert!(s.level[1] >= SEA_LEVEL - 1e-3, "{s:?}");
    }
    // None near the spawn.
    for s in &pieces {
        let near = s.a[0].hypot(s.a[1]).min(s.b[0].hypot(s.b[1]));
        assert!(near > 300.0 || s.width.iter().all(|&w| w < 1.0), "{s:?}");
    }
}

#[test]
fn a_river_is_carved_into_the_ground_and_holds_water() {
    let generator = WorldGenerator::new(1);
    let s = a_river(&generator);
    // Rivers wind off their pieces' straight lines: the deepest water
    // near the middle of the piece.
    let (mx, mz) = ((s.a[0] + s.b[0]) * 0.5, (s.a[1] + s.b[1]) * 0.5);
    let (mut x, mut z, mut depth) = (mx, mz, f32::MIN);
    for k in 0..=24 * 24 {
        let (px, pz) = (mx + (k % 25) as f64 * 2.0 - 24.0, mz + (k / 25) as f64 * 2.0 - 24.0);
        let d = generator.water_level(px, pz) - generator.surface_height(px, pz);
        if d > depth {
            (x, z, depth) = (px, pz, d);
        }
    }
    assert!(depth > 0.5, "{depth} m deep at most, near {mx}, {mz}");
    // Its column holds the carved ground, to the bit.
    let c = ChunkCoord::containing(x, 0.0, z);
    let column = generator.column(c.x, c.z);
    let [ox, _, oz] = c.origin();
    for dz in -12..=12 {
        for dx in -12..=12 {
            let (wx, wz) = (x.floor() + dx as f64, z.floor() + dz as f64);
            let (lx, lz) = ((wx - ox) as i32, (wz - oz) as i32);
            if lx < -COLUMN_MARGIN || lz < -COLUMN_MARGIN || lx > 128 + COLUMN_MARGIN || lz > 128 + COLUMN_MARGIN {
                continue;
            }
            assert_eq!(column.height(lx, lz).to_bits(), generator.surface_height(wx, wz).to_bits());
        }
    }
}

#[test]
fn lakes_lie_in_basins_with_dry_shores() {
    let generator = WorldGenerator::new(1);
    let plan = generator.hydrology().unwrap();
    let (mut checked, mut floors) = (0, 0);
    for ([x, z], level) in plan.lake_cells().step_by(37) {
        let inside = plan.lake_at(x, z).unwrap().1;
        if inside < 0.99 {
            continue;
        }
        checked += 1;
        // Mostly floor: the land makes the shore, and may rise out of the
        // water as an island.
        floors += (generator.surface_height(x, z) < level - 1.0) as i32;
        // The highest of the cells around (their levels differ by
        // centimetres where the basin drains).
        assert!((generator.water_level(x, z) - level.max(SEA_LEVEL)).abs() < 0.5);
    }
    assert!(checked > 20, "{checked} lake cells checked");
    assert!(floors * 10 > checked * 7, "{floors} of {checked} lake cells under water");
    // Where the lake's water ends, the ground stands above it.
    let mut shores = 0;
    for ([x, z], _) in plan.lake_cells().step_by(11) {
        for (dx, dz) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
            for k in 0..256 {
                let (px, pz) = (x + dx * k as f64, z + dz * k as f64);
                let Some((level, inside)) = plan.lake_at(px, pz) else { break };
                // The lake's outer edge, where its water ends.
                if inside < hydrology::LAKE_WATER + 0.01 {
                    if inside < hydrology::LAKE_WATER {
                        break;
                    }
                    shores += 1;
                    let ground = generator.surface_height(px, pz);
                    // (A river reaches the lake at its level: it flows; and
                    // beside one that runs in or out, the two share their
                    // banks under the lake's water.)
                    let river = generator.water_flow(px, pz) != [0.0; 2]
                        || plan.nearest(generator.rivers_at(px, pz), px, pz).is_some_and(|r| (r.level - level).abs() < 0.3);
                    // Dry, but where a river runs in or out.
                    assert!(ground >= level || river, "spill at {px}, {pz}: ground {ground}, lake {level}");
                    break;
                }
            }
        }
    }
    assert!(shores > 20, "{shores} shores");
}

#[test]
fn a_world_without_rivers_has_none() {
    let settings = WorldSettings { rivers: false, ..WorldSettings::default() };
    let generator = WorldGenerator::with_settings(1, settings);
    assert!(generator.hydrology().is_none());
    let with = WorldGenerator::new(1);
    let s = a_river(&with);
    let (x, z) = (s.a[0], s.a[1]);
    assert_eq!(generator.water_level(x, z), SEA_LEVEL);
}

#[test]
#[ignore]
fn river_stats() {
    for seed in [1, 2, 3] {
        let generator = WorldGenerator::new(seed);
        let start = Instant::now();
        let plan = generator.hydrology().unwrap();
        println!("seed {seed}: {:?} in {:?}, max flow {}", plan.counts(), start.elapsed(), plan.max_flow());
        if seed == 1 {
            let s = a_river(&generator);
            println!("river {s:?}");
            let mut wide: Vec<_> = plan.pieces().into_iter().filter(|s| s.width[0] > 20.0).collect();
            wide.sort_by(|a, b| a.a[0].hypot(a.a[1]).total_cmp(&b.a[0].hypot(b.a[1])));
            for s in wide.iter().take(3) {
                println!("wide {s:?}");
            }
            let lake = plan.lake_cells().min_by(|a, b| a.0[0].hypot(a.0[1]).total_cmp(&b.0[0].hypot(b.0[1])));
            println!("lake {lake:?}");
            let start = Instant::now();
            let roads = generator.roads().unwrap();
            println!("roads planned in {:?}", start.elapsed());
            println!("{} settlements, {} roads", roads.settlements().len(), roads.ways().len());
            for s in roads.settlements().iter().take(4) {
                println!("settlement {s:?}");
            }
            let way = &roads.ways()[0];
            println!("road {:?} .. {:?}, mid {:?}", way[0], way.last().unwrap(), way[way.len() / 2]);
        }
    }
}

#[test]
fn settlements_stand_apart_on_land_and_one_at_the_spawn() {
    let generator = WorldGenerator::new(1);
    let roads = generator.roads().unwrap();
    let settlements = roads.settlements();
    assert!(settlements.len() > 15, "{} settlements", settlements.len());
    assert_eq!(settlements[0].centre, [0.0, 0.0]);
    for (i, a) in settlements.iter().enumerate() {
        assert!(a.level > SEA_LEVEL + 1.0, "{a:?}");
        let h = generator.surface_height(a.centre[0], a.centre[1]);
        assert!((h - a.level).abs() < 0.05, "{a:?}: ground {h}");
        for b in &settlements[i + 1..] {
            assert!((a.centre[0] - b.centre[0]).hypot(a.centre[1] - b.centre[1]) > 2000.0);
        }
    }
}

#[test]
fn roads_join_settlements_and_lie_on_the_ground() {
    let generator = WorldGenerator::new(1);
    let roads = generator.roads().unwrap();
    let ways = roads.ways();
    // Some stand on islands of their own.
    assert!(ways.len() * 4 >= roads.settlements().len() * 3, "{} roads", ways.len());
    let is_settlement = |p: [f64; 2]| roads.settlements().iter().any(|s| s.centre == p);
    let (mut on, mut checked) = (0, 0);
    let (mut gentle, mut pieces) = (0, 0);
    for (w, way) in ways.iter().enumerate() {
        assert!(is_settlement(way[0].0) && is_settlement(way.last().unwrap().0));
        for pair in way.windows(2) {
            let ((a, la), (b, lb)) = (pair[0], pair[1]);
            let run = (a[0] - b[0]).hypot(a[1] - b[1]) as f32;
            // Steeper than a road should climb only where the ground forces
            // it, and never very steep.
            assert!((la - lb).abs() <= 0.4 * run + 1e-3, "steep: {pair:?}");
            pieces += 1;
            gentle += ((la - lb).abs() <= 0.121 * run + 1e-3) as i32;
        }
        for (k, &([x, z], level)) in way.iter().enumerate().step_by(23) {
            // Where no river crosses (the road fords it), and not in a
            // tunnel.
            if generator.hydrology().unwrap().river_near(x, z, 40.0) || roads.in_tunnel(w, k) {
                continue;
            }
            checked += 1;
            let h = generator.surface_height(x, z);
            on += ((h - level).abs() < 0.5) as i32;
            let c = ChunkCoord::containing(x, 0.0, z);
            let [ox, _, oz] = c.origin();
            let [road, _, _] = generator.column(c.x, c.z).paint((x - ox) as f32, (z - oz) as f32);
            assert!(road > 0.9, "{road} road at {x}, {z}");
        }
    }
    assert!(checked > 50 && on as f32 > 0.95 * checked as f32, "{on} of {checked}");
    assert!(gentle as f32 > 0.85 * pieces as f32, "{gentle} of {pieces} gentle");
}

#[test]
fn roads_keep_near_the_ground() {
    // No long ramps or deep cuttings: the road's surface stays near the
    // ground's own height (with the rivers' beds and the settlements).
    let generator = WorldGenerator::new(1);
    let roads = generator.roads().unwrap();
    let mut off = Vec::new();
    for (w, way) in roads.ways().iter().enumerate() {
        for (k, &(p, level)) in way.iter().enumerate().step_by(5) {
            // (In a tunnel the road is under the ground, of course.)
            if roads.in_tunnel(w, k) {
                continue;
            }
            let m = generator.interpolated(p[0], p[1], &mut |tx, tz| generator.tile(tx, tz));
            let h = m.height + m.hills * generator.shape.detail(p[0], p[1]);
            let (h, _) = generator.shaped_by_water(generator.near_point(p[0], p[1]).rivers, p[0], p[1], h);
            let (h, _) = Roads::level_settlements(roads.settlements(), p[0], p[1], h);
            off.push((level - h).abs());
        }
    }
    off.sort_by(|a, b| a.total_cmp(b));
    let at = |f: f64| off[((off.len() - 1) as f64 * f) as usize];
    assert!(at(0.9) < 8.0 && at(0.99) < 16.0 && at(1.0) < 45.0, "p90 {} p99 {} max {}", at(0.9), at(0.99), at(1.0));
}

#[test]
fn no_walls_beside_roads() {
    // Where two pieces of road at different heights come close (a hairpin,
    // a junction) and on steep hillsides, the ground beside the road stays
    // a slope, not a wall.
    let generator = WorldGenerator::new(2);
    let roads = generator.roads().unwrap();
    let (mut walls, mut samples, mut worst) = (0, 0, 0.0f32);
    for (w, way) in roads.ways().iter().enumerate() {
        for (k, &(p, _)) in way.iter().enumerate().step_by(40) {
            // Over a tunnel lies the mountain as it is.
            if roads.in_tunnel(w, k) {
                continue;
            }
            for dz in -20..20 {
                for dx in -20..20 {
                    let (x, z) = (p[0].round() + dx as f64, p[1].round() + dz as f64);
                    let h = generator.surface_height(x, z);
                    let step = (generator.surface_height(x + 1.0, z) - h)
                        .abs()
                        .max((generator.surface_height(x, z + 1.0) - h).abs());
                    samples += 1;
                    walls += (step > 4.0) as i32;
                    worst = worst.max(step);
                }
            }
        }
    }
    assert!(walls * 2000 < samples && worst < 8.0, "{walls} walls of {samples}, worst {worst} m per metre");
}

#[test]
fn roads_do_not_cross_themselves() {
    let crosses = |a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]| {
        let o = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);
        o(a, b, c) * o(a, b, d) < 0.0 && o(c, d, a) * o(c, d, b) < 0.0
    };
    let generator = WorldGenerator::new(2);
    for way in generator.roads().unwrap().ways() {
        for i in 0..way.len() - 1 {
            for j in i + 2..way.len() - 1 {
                assert!(
                    !crosses(way[i].0, way[i + 1].0, way[j].0, way[j + 1].0),
                    "crosses itself at {:?}",
                    way[i].0
                );
            }
        }
    }
}

#[test]
fn tunnels_are_tubes_through_the_rock() {
    // Seed 2 has tunnels. In each: air over the floor up to a person's
    // height and more along its middle, rock under the floor, and where
    // enough ground lies over it, rock over the arch; at its ends it opens
    // into the cutting, on the ground at the road's height.
    let generator = WorldGenerator::new(2);
    let tunnels = generator.roads().unwrap().tunnels();
    assert!(tunnels.len() >= 3, "{} tunnels", tunnels.len());
    for tube in tunnels {
        let length: f64 = tube.iter().map(|s| (s.a[0] - s.b[0]).hypot(s.a[1] - s.b[1])).sum();
        assert!(length > 100.0, "a tube {length} m long");
        for s in tube {
            for t in [0.0, 0.5, 1.0] {
                let (x, z) = (s.a[0] + (s.b[0] - s.a[0]) * t, s.a[1] + (s.b[1] - s.a[1]) * t);
                let floor = s.level[0] + (s.level[1] - s.level[0]) * t as f32;
                for up in [0.3, 1.0, 2.0, 4.5] {
                    let d = generator.density(x, (floor + up) as f64, z);
                    assert!(d > 0.0, "no air {up} m over the floor at {x}, {z}: {d}");
                }
                assert!(generator.density(x, (floor - 0.5) as f64, z) < 0.0, "no floor at {x}, {z}");
            }
        }
        // Inside (away from the portals, as far as the cutting's slopes
        // reach, which come down over them on a steep mountainside): at
        // least 1.5 m of rock over the arch.
        let (start, end) = (tube[0].a, tube[tube.len() - 1].b);
        for s in &tube[1..tube.len() - 1] {
            let (x, z) = ((s.a[0] + s.b[0]) * 0.5, (s.a[1] + s.b[1]) * 0.5);
            if (x - start[0]).hypot(z - start[1]).min((x - end[0]).hypot(z - end[1])) < 36.0 {
                continue;
            }
            let top = 0.5 * (s.level[0] + s.level[1]) + TUNNEL_WALL + TUNNEL_HALF_WIDTH;
            assert!(generator.density(x, (top + 1.5) as f64, z) < 0.0, "no rock over the arch at {x}, {z}");
        }
        let (first, last) = (tube[0], tube[tube.len() - 1]);
        for (p, level) in [(first.a, first.level[0]), (last.b, last.level[1])] {
            let h = generator.surface_height(p[0], p[1]);
            assert!((h - level).abs() < 1.5, "the portal at {p:?}: ground {h}, road {level}");
        }
    }
}

#[test]
fn rivers_run_into_the_sea_without_dykes() {
    // Around every river's end in the sea, the ground under the sea (as
    // the land lies without the river) stays under it once the rivers have
    // shaped it: a river cuts its bed, but raises no banks out of the sea,
    // and its bed fades out on the sea floor.
    let generator = WorldGenerator::new(2);
    let plan = generator.hydrology().unwrap();
    let plain = |x: f64, z: f64| {
        let m = generator.interpolated(x, z, &mut |tx, tz| generator.tile(tx, tz));
        m.height + m.hills * generator.shape.detail(x, z)
    };
    let mut checked = 0;
    for s in plan.pieces() {
        // Pieces level at the sea's level: the ends that run into it.
        if s.level[0] > SEA_LEVEL + 0.01 || s.level[1] > SEA_LEVEL + 0.01 {
            continue;
        }
        let m = [(s.a[0] + s.b[0]) * 0.5, (s.a[1] + s.b[1]) * 0.5];
        for j in -20..=20 {
            for i in -20..=20 {
                let (x, z) = (m[0] + 3.0 * i as f64, m[1] + 3.0 * j as f64);
                let ground = plain(x, z);
                if ground < SEA_LEVEL - 0.2 {
                    checked += 1;
                    // As the rivers (and lakes) shape it: settlements and
                    // roads by the sea may raise their ground on purpose.
                    let rivers = generator.near_point(x, z).rivers;
                    let (h, _) = generator.shaped_by_water(rivers, x, z, ground);
                    assert!(h < SEA_LEVEL, "a dyke at {x}, {z}: {h} m over the sea");
                    // Out on the sea floor, deeper than a river's bed, the
                    // bed has faded out: no trench (and no trail of gravel)
                    // runs on into the sea.
                    if ground < SEA_LEVEL - 6.5 {
                        assert_eq!(h, ground, "a trench at {x}, {z}");
                    }
                }
            }
        }
    }
    assert!(checked > 1000, "{checked} points checked");
}

#[test]
fn rivers_only_cut_the_ground() {
    for h in [-30.0f32, -2.0, 0.0, 0.5, 3.0, 20.0] {
        for distance in [0.0f32, 2.0, 5.0, 12.0, 40.0] {
            let river = hydrology::NearestRiver {
                distance,
                level: 1.0,
                width: 8.0,
                direction: [1.0, 0.0],
                grade: 0.0,
            };
            let carved = Hydrology::carve(h, Some(river));
            assert!(
                carved <= h,
                "ground {h} raised to {carved} {distance} m from a river"
            );
        }
    }
}

#[test]
fn narrow_rivers_hold_water_at_every_metre_along_them() {
    // The terrain's mesh samples the ground at whole metres: around a
    // narrow river, the points whose ground lies under its water are
    // joined side by side (not only corner to corner), or the mesh catches
    // the bed in pieces and the water shows in pieces. A river runs out
    // of the window it is looked at in; a piece that lies within it is a
    // break (two rivers meeting beyond it are two pieces, but not breaks).
    let mut checked = 0;
    for seed in [1, 2, 3] {
        let generator = WorldGenerator::new(seed);
        let plan = generator.hydrology().unwrap();
        for s in plan.pieces() {
            let w = s.width[0].max(s.width[1]);
            if w > 3.4 || s.width[0].min(s.width[1]) < 1.0 || s.level[1] < SEA_LEVEL + 1.0 {
                continue;
            }
            let m = [
                ((s.a[0] + s.b[0]) * 0.5).floor(),
                ((s.a[1] + s.b[1]) * 0.5).floor(),
            ];
            const R: i32 = 20;
            let n = 2 * R + 1;
            let wet: Vec<bool> = (0..n * n)
                .map(|k| {
                    let (x, z) = (m[0] + (k % n - R) as f64, m[1] + (k / n - R) as f64);
                    generator.water_level(x, z) - generator.surface_height(x, z) > 0.3
                })
                .collect();
            // Pieces of joined wet points that do not reach the window's
            // edge, but the smallest.
            let mut seen = vec![false; wet.len()];
            let (mut pieces, mut breaks) = (0, 0);
            for start in 0..wet.len() {
                if !wet[start] || seen[start] {
                    continue;
                }
                let (mut stack, mut size, mut edge) = (vec![start], 0, false);
                seen[start] = true;
                while let Some(k) = stack.pop() {
                    size += 1;
                    let (i, j) = (k as i32 % n, k as i32 / n);
                    edge |= i == 0 || j == 0 || i == n - 1 || j == n - 1;
                    for (di, dj) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                        let (a, b) = (i + di, j + dj);
                        if (0..n).contains(&a) && (0..n).contains(&b) {
                            let q = (b * n + a) as usize;
                            if wet[q] && !seen[q] {
                                seen[q] = true;
                                stack.push(q);
                            }
                        }
                    }
                }
                if size > 3 {
                    pieces += 1;
                    breaks += !edge as i32;
                }
            }
            if pieces == 0 {
                continue;
            }
            checked += 1;
            assert_eq!(breaks, 0, "seed {seed}: a river {w} m wide around {m:?} breaks into {pieces} pieces");
        }
    }
    assert!(checked >= 3, "{checked}");
}

#[test]
fn rivers_run_out_of_their_lakes() {
    // From a lake's water, the water is joined (side by side, at whole
    // metres, as the terrain's mesh sees it) to the river running out of
    // it: no ground between the two.
    let mut checked = 0;
    for seed in [1, 2, 3] {
        let generator = WorldGenerator::new(seed);
        let plan = generator.hydrology().unwrap();
        for &(from, to) in plan.outlets() {
            let wet = |x: f64, z: f64| generator.water_level(x, z) - generator.surface_height(x, z) > 0.2;
            let (x0, z0) = (from[0].min(to[0]).floor() - 40.0, from[1].min(to[1]).floor() - 40.0);
            let (w, h) = ((from[0] - to[0]).abs() as i32 + 81, (from[1] - to[1]).abs() as i32 + 81);
            let at = |k: i32| (x0 + (k % w) as f64, z0 + (k / w) as f64);
            let near = |k: i32, p: [f64; 2], r: f64| {
                let (x, z) = at(k);
                (x - p[0]).hypot(z - p[1]) < r
            };
            // Where the lake's water is: points around `from`.
            let start: Vec<i32> = (0..w * h).filter(|&k| near(k, from, 6.0) && wet(at(k).0, at(k).1)).collect();
            // The river's water around `to`, moving.
            let river: Vec<i32> = (0..w * h)
                .filter(|&k| {
                    let (x, z) = at(k);
                    near(k, to, 24.0) && wet(x, z) && generator.water_flow(x, z) != [0.0; 2]
                })
                .collect();
            if start.is_empty() || river.is_empty() {
                continue;
            }
            let mut seen = vec![false; (w * h) as usize];
            let mut stack = start.clone();
            for &k in &start {
                seen[k as usize] = true;
            }
            while let Some(k) = stack.pop() {
                let (i, j) = (k % w, k / w);
                for (di, dj) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let (a, b) = (i + di, j + dj);
                    if (0..w).contains(&a) && (0..h).contains(&b) {
                        let q = b * w + a;
                        if !seen[q as usize] && wet(at(q).0, at(q).1) {
                            seen[q as usize] = true;
                            stack.push(q);
                        }
                    }
                }
            }
            checked += 1;
            assert!(
                river.iter().any(|&k| seen[k as usize]),
                "seed {seed}: the river out of the lake at {from:?} starts apart from it, at {to:?}"
            );
        }
    }
    assert!(checked >= 3, "{checked}");
}

#[test]
fn lake_shores_are_mostly_gentle() {
    // Around the lakes' waterlines the ground mostly rises gently; a steep
    // bank may stand where the land around is steep anyway, but the lake
    // does not cut cliffs into it.
    let generator = WorldGenerator::new(1);
    let plan = generator.hydrology().unwrap();
    let (mut steep, mut samples) = (0, 0);
    for ([x, z], _) in plan.lake_cells().step_by(7) {
        for (dx, dz) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
            for k in 0..96 {
                let (px, pz) = (x + dx * k as f64, z + dz * k as f64);
                let Some((level, inside)) = plan.lake_at(px, pz) else { break };
                if inside < 0.05 {
                    break;
                }
                let h = generator.surface_height(px, pz);
                // Within 3 m of the lake's level: the waterline and the
                // shore just over it.
                if (h - level).abs() > 3.0 {
                    continue;
                }
                samples += 1;
                let slope = (generator.surface_height(px + dx, pz + dz) - h).abs();
                steep += (slope > 1.0) as i32;
            }
        }
    }
    assert!(samples > 500, "{samples}");
    assert!(steep * 10 < samples, "{steep} of {samples} shore samples steeper than 45 degrees");
}
