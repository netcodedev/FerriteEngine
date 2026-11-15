use std::{sync::Arc, thread, time::Instant};

use libnoise::prelude::*;

use super::*;

#[test]
fn the_ground_is_the_height_field_it_always_was() {
    // The terrain before the generator existed, written out: a chunk
    // looks the same after the move.
    let seed = 2;
    let noise = Source::perlin(seed).scale([0.003; 2]).fbm(6, 1.0, 2.0, 0.5);
    let old = |x: f64, z: f64| {
        let offset = 16777216.0;
        ((1.0 + noise.sample([x + offset, z + offset])) / 2.0) as f32 * CHUNK_SIZE_FLOAT - 1.0
    };
    let generator = WorldGenerator::new(seed);
    let column = generator.column(-3, 2);
    for (x, z) in [(-9, -9), (0, 0), (17, 100), (137, 137), (64, -5)] {
        let expected = old(-3.0 * 128.0 + x as f64, 2.0 * 128.0 + z as f64);
        assert_eq!(column.height(x, z).to_bits(), expected.to_bits(), "({x}, {z})");
    }
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
    assert!(generator.columns.lock().unwrap().columns.len() <= COLUMN_CACHE);
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
