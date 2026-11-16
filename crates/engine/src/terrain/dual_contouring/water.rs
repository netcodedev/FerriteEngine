//! The water surface of a terrain chunk.
//!
//! The surface is a regular grid at the water's level (the sea's, or a
//! lake's or river's where one is: see `generator::hydrology`), aligned to world
//! coordinates so neighbouring chunks share their border vertices exactly.
//! It does not stop at the shoreline: every cell whose ground comes near the
//! water is kept, plus a ring of cells around them, so the grid's edge always
//! lies under dry land. Where the water meets the terrain is then decided per
//! pixel by the depth test (and softened in the shader by how deep the water
//! is in front of the ground), never by a mesh edge, so the waves cannot
//! open gaps along the shore.
//!
//! Each vertex carries how deep the water is beneath it (negative under
//! land) and how large the body of water around it is. The shader damps the
//! waves toward the shore with the first and picks the waves the second can
//! carry: ponds only ripple, lakes and the sea get long swells.

use gl::types::GLuint;

use crate::core::renderer::shader::VertexAttributes;

/// Spacing of the surface grid. Small enough for the shortest waves that
/// displace the surface (see `water_vertex.glsl`); shorter ones only bend
/// the normals.
pub const GRID_STEP: f32 = 2.0;
/// Cells whose ground is up to this far above the water still get a
/// surface, so waves running up the beach have water to run with.
const SHORE_MARGIN: f32 = 2.5;
/// Spacing of the points the size of the water body is measured at; the
/// vertices in between interpolate.
const SIZE_STEP: f32 = 16.0;
/// Step and reach of the rays that measure a body of water.
const SIZE_RAY_STEP: f32 = 6.0;
const SIZE_RAY_REACH: f32 = 480.0;

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct WaterVertex {
    pub position: [f32; 3],
    /// Water depth under the vertex; negative under dry land.
    pub depth: f32,
    /// Typical width of the body of water around the vertex, in world
    /// units (0 for a puddle the measurement does not catch).
    pub body_size: f32,
}

impl VertexAttributes for WaterVertex {
    fn get_vertex_attributes() -> Vec<(usize, GLuint)> {
        vec![(3, gl::FLOAT), (1, gl::FLOAT), (1, gl::FLOAT)]
    }
}

/// Build the water surface of the chunk whose corner is at world `(x0, z0)`
/// and which is `size` wide, over the ground given by `ground(x, z)` (world
/// height), where the water would stand at `level(x, z)`. Positions are
/// relative to the chunk's corner and to `floor` (height). `None` when no
/// water comes near the chunk.
pub fn generate(
    ground: impl Fn(f32, f32) -> f32,
    level: impl Fn(f32, f32) -> f32,
    x0: f32,
    z0: f32,
    size: f32,
    floor: f32,
) -> Option<(Vec<WaterVertex>, Vec<u32>)> {
    let cells = (size / GRID_STEP).round() as i32;
    // Heights two vertices beyond the chunk on every side, so the ring of
    // extra cells and the levels along the border are decided the same way
    // by the chunks on both sides.
    let span = cells + 5;
    let height_at = |heights: &[f32], i: i32, j: i32| heights[((j + 2) * span + (i + 2)) as usize];
    let mut heights = Vec::with_capacity((span * span) as usize);
    let mut levels = Vec::with_capacity((span * span) as usize);
    for j in -2..=cells + 2 {
        for i in -2..=cells + 2 {
            let (x, z) = (x0 + i as f32 * GRID_STEP, z0 + j as f32 * GRID_STEP);
            heights.push(ground(x, z));
            levels.push(level(x, z));
        }
    }
    let vertex_wet = |i: i32, j: i32| height_at(&heights, i, j) < height_at(&levels, i, j) + SHORE_MARGIN;
    // A cell is wet when any of its corners is (nearly) under water.
    let wet = |i: i32, j: i32| -> bool { (0..2).any(|dj| (0..2).any(|di| vertex_wet(i + di, j + dj))) };
    // Where the level changes (a river beside the sea's level, say), a dry
    // vertex of a kept cell takes the highest level of the wet ones next to
    // it, so the surface stays level instead of diving into the bank.
    let surface_at = |i: i32, j: i32| -> f32 {
        let own = height_at(&levels, i, j);
        if vertex_wet(i, j) {
            return own;
        }
        // Two vertices out: a kept cell's corners lie that close to a wet
        // vertex (its ring reaches one cell beyond the wet ones).
        let mut best: Option<f32> = None;
        for dj in -2..=2 {
            for di in -2..=2 {
                let (a, b) = (i + di, j + dj);
                if (-2..=cells + 2).contains(&a) && (-2..=cells + 2).contains(&b) && vertex_wet(a, b) {
                    let l = height_at(&levels, a, b);
                    best = Some(best.map_or(l, |m: f32| m.max(l)));
                }
            }
        }
        best.unwrap_or(own)
    };
    // Kept: wet, or next to a wet cell (the ring that tucks the edge under
    // the shore).
    let kept = |i: i32, j: i32| -> bool {
        (-1..=1).any(|dj| (-1..=1).any(|di| wet_in_range(i + di, j + dj, cells) && wet(i + di, j + dj)))
    };

    let mut cell_kept = vec![false; (cells * cells) as usize];
    let mut any = false;
    for j in 0..cells {
        for i in 0..cells {
            if kept(i, j) {
                cell_kept[(j * cells + i) as usize] = true;
                any = true;
            }
        }
    }
    if !any {
        return None;
    }

    let body_size = BodySize::measure(&ground, &level, x0, z0, size);
    let corners = cells + 1;
    let mut index_of = vec![u32::MAX; (corners * corners) as usize];
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut vertex = |i: i32, j: i32, vertices: &mut Vec<WaterVertex>| -> u32 {
        let slot = &mut index_of[(j * corners + i) as usize];
        if *slot == u32::MAX {
            let (x, z) = (i as f32 * GRID_STEP, j as f32 * GRID_STEP);
            *slot = vertices.len() as u32;
            let surface = surface_at(i, j);
            vertices.push(WaterVertex {
                position: [x, surface - floor, z],
                depth: surface - height_at(&heights, i, j),
                body_size: body_size.at(x, z),
            });
        }
        *slot
    };
    for j in 0..cells {
        for i in 0..cells {
            if !cell_kept[(j * cells + i) as usize] {
                continue;
            }
            let a = vertex(i, j, &mut vertices);
            let b = vertex(i + 1, j, &mut vertices);
            let c = vertex(i + 1, j + 1, &mut vertices);
            let d = vertex(i, j + 1, &mut vertices);
            // Counter-clockwise seen from above.
            indices.extend_from_slice(&[a, d, c, a, c, b]);
        }
    }
    Some((vertices, indices))
}

/// Cells one beyond the chunk count too: their heights are known.
fn wet_in_range(i: i32, j: i32, cells: i32) -> bool {
    (-1..=cells).contains(&i) && (-1..=cells).contains(&j)
}

/// Body sizes on a world aligned grid over one chunk.
struct BodySize {
    samples: Vec<f32>,
    points: i32,
}

impl BodySize {
    fn measure(ground: &impl Fn(f32, f32) -> f32, level: &impl Fn(f32, f32) -> f32, x0: f32, z0: f32, size: f32) -> Self {
        let steps = (size / SIZE_STEP).round() as i32;
        // One extra point on every side, for the dilation below.
        let span = steps + 3;
        let mut raw = Vec::with_capacity((span * span) as usize);
        for j in -1..=steps + 1 {
            for i in -1..=steps + 1 {
                let (x, z) = (x0 + i as f32 * SIZE_STEP, z0 + j as f32 * SIZE_STEP);
                raw.push(measure_body(ground, level(x, z), x, z));
            }
        }
        // Points on land take the largest size of the water next to them,
        // so the vertices between them and the water do not fade toward 0.
        let points = steps + 1;
        let mut samples = Vec::with_capacity((points * points) as usize);
        for j in 0..points {
            for i in 0..points {
                let at = |i: i32, j: i32| raw[((j + 1) * span + (i + 1)) as usize];
                let own = at(i, j);
                if own > 0.0 {
                    samples.push(own);
                } else {
                    let mut best: f32 = 0.0;
                    for dj in -1..=1 {
                        for di in -1..=1 {
                            best = best.max(at(i + di, j + dj));
                        }
                    }
                    samples.push(best);
                }
            }
        }
        Self { samples, points }
    }

    /// Bilinear between the samples, at chunk-local `(x, z)`.
    fn at(&self, x: f32, z: f32) -> f32 {
        let fx = (x / SIZE_STEP).clamp(0.0, (self.points - 1) as f32);
        let fz = (z / SIZE_STEP).clamp(0.0, (self.points - 1) as f32);
        let (i, j) = (
            (fx.floor() as i32).min(self.points - 2),
            (fz.floor() as i32).min(self.points - 2),
        );
        let (tx, tz) = (fx - i as f32, fz - j as f32);
        let s = |i: i32, j: i32| self.samples[(j * self.points + i) as usize];
        let top = s(i, j) * (1.0 - tx) + s(i + 1, j) * tx;
        let bottom = s(i, j + 1) * (1.0 - tx) + s(i + 1, j + 1) * tx;
        top * (1.0 - tz) + bottom * tz
    }
}

/// Typical width of the water at world `(x, z)`: the geometric mean of
/// its width along four directions. A long narrow river comes out close to
/// its width rather than its length, which is what limits its waves.
/// 0 on dry land.
fn measure_body(ground: &impl Fn(f32, f32) -> f32, level: f32, x: f32, z: f32) -> f32 {
    if ground(x, z) >= level {
        return 0.0;
    }
    let reach = |dx: f32, dz: f32| -> f32 {
        let mut t = SIZE_RAY_STEP;
        while t < SIZE_RAY_REACH {
            if ground(x + dx * t, z + dz * t) >= level {
                return t;
            }
            t += SIZE_RAY_STEP;
        }
        SIZE_RAY_REACH
    };
    let d = std::f32::consts::FRAC_1_SQRT_2;
    let directions = [(1.0, 0.0), (d, d), (0.0, 1.0), (-d, d)];
    let log_sum: f32 = directions
        .iter()
        .map(|&(dx, dz)| (reach(dx, dz) + reach(-dx, -dz)).ln())
        .sum();
    (log_sum / directions.len() as f32).exp()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::dual_contouring::WATER_LEVEL;

    const SIZE: f32 = 128.0;

    fn sea(_: f32, _: f32) -> f32 {
        WATER_LEVEL
    }

    /// A round lake of `radius` around the origin, with a beach rising
    /// `slope` per unit beyond it.
    fn lake(radius: f32, slope: f32) -> impl Fn(f32, f32) -> f32 {
        move |x: f32, z: f32| {
            let r = (x * x + z * z).sqrt();
            WATER_LEVEL + (r - radius) * slope
        }
    }

    #[test]
    fn dry_chunks_have_no_water() {
        assert!(generate(|_, _| WATER_LEVEL + 20.0, sea, 0.0, 0.0, SIZE, 0.0).is_none());
    }

    #[test]
    fn larger_lakes_measure_larger() {
        let pond = measure_body(&lake(15.0, 0.5), WATER_LEVEL, 0.0, 0.0);
        let big = measure_body(&lake(200.0, 0.5), WATER_LEVEL, 0.0, 0.0);
        assert!(pond > 20.0 && pond < 40.0, "pond {pond}");
        assert!(big > 350.0, "lake {big}");
        assert_eq!(measure_body(&lake(15.0, 0.5), WATER_LEVEL, 40.0, 0.0), 0.0);
    }

    #[test]
    fn narrow_rivers_measure_close_to_their_width() {
        let river = |_x: f32, z: f32| WATER_LEVEL + (z.abs() - 10.0) * 0.5;
        let width = measure_body(&river, WATER_LEVEL, 0.0, 0.0);
        assert!(width < 80.0, "river {width}");
    }

    /// Every edge of the surface that is not on the chunk's border lies
    /// under dry land, so the shoreline is never a mesh edge.
    #[test]
    fn surface_edges_end_under_land() {
        let ground = lake(50.0, 0.3);
        let (x0, z0) = (-64.0, -64.0);
        let (vertices, indices) = generate(&ground, sea, x0, z0, SIZE, 0.0).expect("lake has water");
        let mut edges = std::collections::HashMap::new();
        for tri in indices.chunks(3) {
            for k in 0..3 {
                let (a, b) = (tri[k], tri[(k + 1) % 3]);
                *edges.entry((a.min(b), a.max(b))).or_insert(0) += 1;
            }
        }
        let on_border = |v: &WaterVertex| {
            let [x, _, z] = v.position;
            x <= 0.0 || z <= 0.0 || x >= SIZE || z >= SIZE
        };
        for ((a, b), count) in edges {
            if count != 1 {
                continue;
            }
            let (va, vb) = (&vertices[a as usize], &vertices[b as usize]);
            if on_border(va) && on_border(vb) {
                continue;
            }
            for v in [va, vb] {
                assert!(v.depth < 0.0, "open edge over water at {:?}", v.position);
            }
        }
    }

    /// Chunks next to each other put the same vertices on their shared
    /// border, with the same depth and body size.
    #[test]
    fn neighbours_agree_on_their_border() {
        let ground = lake(150.0, 0.2);
        let (left, _) = generate(&ground, sea, -128.0, 0.0, SIZE, 0.0).unwrap();
        let (right, _) = generate(&ground, sea, 0.0, 0.0, SIZE, 0.0).unwrap();
        let border = |vertices: &[WaterVertex], x: f32| {
            let mut found: Vec<(i32, f32, f32)> = vertices
                .iter()
                .filter(|v| v.position[0] == x)
                .map(|v| (v.position[2] as i32, v.depth, v.body_size))
                .collect();
            found.sort_by_key(|f| f.0);
            found
        };
        let a = border(&left, SIZE);
        let b = border(&right, 0.0);
        assert!(!a.is_empty());
        assert_eq!(a.len(), b.len());
        for (p, q) in a.iter().zip(&b) {
            assert_eq!(p.0, q.0);
            assert!((p.1 - q.1).abs() < 1e-4);
            assert!((p.2 - q.2).abs() < 1e-2, "{p:?} vs {q:?}");
        }
    }

    /// A river high above the sea: its surface lies at its own level, and
    /// the cells beside it stay level with it.
    #[test]
    fn a_river_stands_at_its_own_level() {
        let river_level = 140.0;
        let ground = |_x: f32, z: f32| river_level - 2.0 + (z - 64.0).abs().max(6.0) - 6.0 + 2.0 * ((z - 64.0).abs() / 6.0).min(1.0);
        let level = |_x: f32, z: f32| if (z - 64.0).abs() < 10.0 { river_level } else { WATER_LEVEL };
        let (vertices, _) = generate(ground, level, 0.0, 0.0, SIZE, 128.0).expect("river has water");
        for v in &vertices {
            assert!((v.position[1] - (river_level - 128.0)).abs() < 1e-4, "{:?}", v.position);
        }
    }
}
