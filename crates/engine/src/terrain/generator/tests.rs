use std::{sync::Arc, thread, time::Instant};

use super::*;

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
    generator.hydrology();
    let n = 16;
    let start = Instant::now();
    for i in 0..n {
        generator.make_column(i, 1000);
    }
    let column = start.elapsed() / n as u32;
    // Inland, by rivers (seed 1).
    let inland = WorldGenerator::new(1);
    inland.hydrology();
    let start = Instant::now();
    for i in 0..n {
        inland.make_column(20 + i, 14);
    }
    println!("inland column: {:?}", start.elapsed() / n as u32);
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
    // A mountain column with a known cave 40 m under its surface.
    let (x, z) = (-2345.0, -5892.0);
    let surface = generator.surface_height(x, z) as f64;
    assert!(generator.density(x, surface - 40.0, z) > 0.0, "no cave");
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
    let mut checked = 0;
    for ([x, z], level) in plan.lake_cells().step_by(37) {
        let inside = plan.lake_at(x, z).unwrap().1;
        if inside < 0.99 {
            continue;
        }
        checked += 1;
        assert!(generator.surface_height(x, z) < level - 1.0, "lake floor at {x}, {z}");
        // The highest of the cells around (their levels differ by
        // centimetres where the basin drains).
        assert!((generator.water_level(x, z) - level.max(SEA_LEVEL)).abs() < 0.5);
    }
    assert!(checked > 20, "{checked} lake cells checked");
    // Where the lake's water ends, the ground stands above it.
    let mut shores = 0;
    for ([x, z], _) in plan.lake_cells().step_by(11) {
        for (dx, dz) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
            for k in 0..256 {
                let (px, pz) = (x + dx * k as f64, z + dz * k as f64);
                let Some((level, inside)) = plan.lake_at(px, pz) else { break };
                // The lake's outer edge, where its water ends.
                if inside < hydrology::LAKE_WATER + 0.05 {
                    if inside < hydrology::LAKE_WATER {
                        break;
                    }
                    shores += 1;
                    let ground = generator.surface_height(px, pz);
                    let river = generator.water_level(px, pz) > generator.still_water_level(px, pz);
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
        }
    }
}
