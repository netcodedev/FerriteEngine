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
    let n = 16;
    let start = Instant::now();
    for i in 0..n {
        generator.make_column(i, 1000);
    }
    let column = start.elapsed() / n as u32;
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
    let lattice = generator.underground(coord).expect("caves in the chunk");
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
    assert!(generator.underground(ChunkCoord::new(-19, 1, -46)).is_none());
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
            for cy in DualContouringChunk::vertical_range(&generator, (cx, cz)) {
                let t = Instant::now();
                let _ = generator.underground(ChunkCoord::new(cx, cy, cz));
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
