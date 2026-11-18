//! Dual contouring of a chunk's density field: one vertex in every cell
//! the surface passes through, placed where the planes of the surface at
//! the cell's edges meet best (a least-squares fit, the cell's "QEF"), and
//! one quad across every edge the surface crosses.
//!
//! Where the surface is smooth the planes nearly agree and the vertex
//! settles on them near the middle of the crossings, as surface nets would
//! put it; where they disagree (a ridge, a ledge, the corner of a block) the
//! vertex goes to where they meet, so the edge stays sharp instead of being
//! rounded off. How sharp depends on how well the planes are known: here
//! they come from the field's samples (a whole metre apart), so features
//! smaller than a cell still round.
//!
//! Coarse levels take their crossings and planes from the full-detail
//! field along their (longer) edges, so their vertices lie on the same
//! ground as the finest level's.

use std::collections::HashMap;

use super::{
    mesher::{DensityField, TerrainVertex},
    source::ChunkSurface,
    CHUNK_SIZE,
};

/// Directions of the fit (eigenvalues of its normal matrix) weaker than
/// this share of the strongest one count as unknown: along them the vertex
/// stays at the middle of the crossings. Two planes meeting at an angle `a`
/// give a share of about tan²(a / 2): this keeps edges from about 40° sharp
/// (ridges, corners, the cracks of rock) and leaves ground curved within a
/// cell round, which the fit would otherwise break into flat facets meeting
/// in creases.
const TRUNCATION: f32 = 0.13;

/// Where the field has a sharp part (see `ChunkField::fill_volume_layered`):
/// how many pieces an edge is searched in for the surface's first crossing,
/// how often that crossing is then narrowed down, and the step of the
/// differences that give its normal (metres).
const SHARP_SEARCH: i32 = 6;
const SHARP_REFINE: usize = 5;
const SHARP_NORMAL_STEP: f32 = 0.01;

/// Crossings worked out exactly (with the sharp part), by edge: its first
/// sample (chunk-local) and its axis. Each edge borders four cells.
type ExactCrossings = HashMap<([i32; 3], usize), Hermite>;

/// A point where the surface crosses an edge, and the surface's unit
/// normal there.
#[derive(Clone, Copy, Debug)]
pub struct Hermite {
    pub point: [f32; 3],
    pub normal: [f32; 3],
}

impl DensityField<'_> {
    /// The surface of the chunk's cube sampled `step` metres apart, with
    /// one sample past each side (as `surface_nets`, whose output it can
    /// replace): vertices (chunk-local, normals from the full-detail field)
    /// and triangles facing out of the ground. `None` where the cube holds
    /// no surface.
    pub fn dual_contouring(&self, step: i32, surface: &dyn ChunkSurface) -> Option<(Vec<TerrainVertex>, Vec<u32>)> {
        let n = CHUNK_SIZE as i32 / step + 3;
        let cells = n - 1;
        let local = |i: i32| (i - 1) * step;
        // The samples of one layer of the grid (x fastest).
        let layer = |z: i32| -> Vec<f32> {
            let mut values = Vec::with_capacity((n * n) as usize);
            for y in 0..n {
                for x in 0..n {
                    values.push(self.at(local(x), local(y), local(z)));
                }
            }
            values
        };
        // The vertex of each cell of this layer of cells and the one below
        // (quads reach back one cell along every axis).
        let mut vertex_of = [vec![u32::MAX; (cells * cells) as usize], vec![u32::MAX; (cells * cells) as usize]];
        let mut positions: Vec<[f32; 3]> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut exact = ExactCrossings::new();
        let sharp = step == 1 && self.field.has_sharp() && surface.exact_sharp();
        let mut below = layer(0);
        for z in 0..cells {
            let above = layer(z + 1);
            let (first, second) = vertex_of.split_at_mut(1);
            let (current, previous) = if z % 2 == 0 {
                (&mut first[0], &second[0])
            } else {
                (&mut second[0], &first[0])
            };
            current.fill(u32::MAX);
            for y in 0..cells {
                for x in 0..cells {
                    let mut values = [0.0f32; 8];
                    let mut inside = 0u8;
                    for (corner, value) in values.iter_mut().enumerate() {
                        let (dx, dy, dz) = (corner as i32 & 1, (corner as i32 >> 1) & 1, (corner as i32 >> 2) & 1);
                        let samples = if dz == 0 { &below } else { &above };
                        *value = samples[((y + dy) * n + x + dx) as usize];
                        if *value < 0.0 {
                            inside |= 1 << corner;
                        }
                    }
                    if inside == 0 || inside == 0xff {
                        continue;
                    }
                    let corner = [x, y, z].map(local);
                    current[(y * cells + x) as usize] = positions.len() as u32;
                    let vertex = if sharp && self.has_sharp_corner(corner) {
                        self.sharp_cell_vertex(corner, &values, surface, &mut exact)
                    } else {
                        self.cell_vertex(corner, step, &values)
                    };
                    positions.push(vertex);
                    // A quad across each edge from the cell's first corner
                    // the surface crosses, joining the four cells around
                    // it (this one and three made before).
                    for axis in 0..3 {
                        // Corner 0 and the corner one step along `axis`
                        // (corner `1 << axis`).
                        let start_inside = inside & 1 != 0;
                        if start_inside == (inside & (1 << (1 << axis)) != 0) {
                            continue;
                        }
                        let (b, c) = ((axis + 1) % 3, (axis + 2) % 3);
                        let p = [x, y, z];
                        if p[b] == 0 || p[c] == 0 {
                            continue;
                        }
                        let cell = |back_b: i32, back_c: i32| -> u32 {
                            let mut q = p;
                            q[b] -= back_b;
                            q[c] -= back_c;
                            let slab = if q[2] == z { &*current } else { previous };
                            slab[(q[1] * cells + q[0]) as usize]
                        };
                        // Round the edge, counter-clockwise seen from its
                        // end: the quad faces along the edge.
                        let mut quad = [cell(1, 1), cell(0, 1), cell(0, 0), cell(1, 0)];
                        if quad.contains(&u32::MAX) {
                            continue;
                        }
                        // It should face out of the ground: towards the
                        // edge's end if its start is inside.
                        if !start_inside {
                            quad.reverse();
                        }
                        let [q0, q1, q2, q3] = quad;
                        let distance = |i: u32, j: u32| -> f32 {
                            let (a, b) = (positions[i as usize], positions[j as usize]);
                            (0..3).map(|k| (a[k] - b[k]).powi(2)).sum()
                        };
                        // Split along the shorter diagonal.
                        if distance(q0, q2) <= distance(q1, q3) {
                            indices.extend_from_slice(&[q0, q1, q2, q0, q2, q3]);
                        } else {
                            indices.extend_from_slice(&[q0, q1, q3, q1, q2, q3]);
                        }
                    }
                }
            }
            below = above;
        }
        if indices.is_empty() {
            return None;
        }
        let vertices = positions.into_iter().map(|p| self.painted(p, surface)).collect();
        Some((vertices, indices))
    }

    /// The vertex of the cell with its first corner at chunk-local
    /// `corner`, `step` metres a side, whose corners hold `values` (x
    /// fastest, then y, then z): the best fit to the surface's planes at
    /// the cell's crossed edges, kept in the cell.
    fn cell_vertex(&self, corner: [i32; 3], step: i32, values: &[f32; 8]) -> [f32; 3] {
        let mut crossings = [Hermite { point: [0.0; 3], normal: [0.0; 3] }; 12];
        let mut count = 0;
        for axis in 0..3 {
            let (b, c) = ((axis + 1) % 3, (axis + 2) % 3);
            for side in 0..4 {
                let (on_b, on_c) = (side & 1, side >> 1);
                let start = (on_b << b) | (on_c << c);
                let end = start | (1 << axis);
                if (values[start] < 0.0) == (values[end] < 0.0) {
                    continue;
                }
                let mut from = corner;
                from[b] += on_b as i32 * step;
                from[c] += on_c as i32 * step;
                crossings[count] = self.crossing(from, axis, step, [on_b == 1, on_c == 1]);
                count += 1;
            }
        }
        in_cell(&crossings[..count], corner.map(|c| c as f32), corner.map(|c| (c + step) as f32))
    }

    /// Whether a corner of the full-detail cell with its first corner at
    /// `corner` has a sharp part.
    fn has_sharp_corner(&self, corner: [i32; 3]) -> bool {
        (0..8).any(|k| self.field.sharp_at(corner[0] + (k & 1), corner[1] + ((k >> 1) & 1), corner[2] + ((k >> 2) & 1)) != 0.0)
    }

    /// As `cell_vertex` for a full-detail cell with a sharp part: the
    /// crossings and normals from the field's smooth rest blended from the
    /// cell's corners plus the sharp part exactly (see `exact_crossing`).
    fn sharp_cell_vertex(
        &self,
        corner: [i32; 3],
        values: &[f32; 8],
        surface: &dyn ChunkSurface,
        exact: &mut ExactCrossings,
    ) -> [f32; 3] {
        let mut crossings = [Hermite { point: [0.0; 3], normal: [0.0; 3] }; 12];
        let mut count = 0;
        for axis in 0..3 {
            let (b, c) = ((axis + 1) % 3, (axis + 2) % 3);
            for side in 0..4 {
                let (on_b, on_c) = (side & 1, side >> 1);
                let start = (on_b << b) | (on_c << c);
                let end = start | (1 << axis);
                if (values[start] < 0.0) == (values[end] < 0.0) {
                    continue;
                }
                let mut from = corner;
                from[b] += on_b as i32;
                from[c] += on_c as i32;
                crossings[count] = *exact
                    .entry((from, axis))
                    .or_insert_with(|| self.exact_crossing(from, axis, surface));
                count += 1;
            }
        }
        in_cell(&crossings[..count], corner.map(|c| c as f32), corner.map(|c| c as f32 + 1.0))
    }

    /// The density at chunk-local `p` with its sharp part exact: the rest
    /// blended from the samples around `p`, plus `surface.sharp(p)`.
    fn exact_value(&self, p: [f32; 3], surface: &dyn ChunkSurface) -> f32 {
        let base = p.map(|c| c.floor() as i32);
        let t: [f32; 3] = std::array::from_fn(|i| p[i] - base[i] as f32);
        let mut rest = 0.0;
        for corner in 0..8 {
            let d = [corner & 1, (corner >> 1) & 1, (corner >> 2) & 1];
            let weight: f32 = (0..3).map(|i| if d[i] == 1 { t[i] } else { 1.0 - t[i] }).product();
            if weight != 0.0 {
                let (x, y, z) = (base[0] + d[0], base[1] + d[1], base[2] + d[2]);
                rest += weight * (self.at(x, y, z) - self.field.sharp_at(x, y, z));
            }
        }
        rest + surface.sharp(p)
    }

    /// Where the surface crosses the full-detail edge from chunk-local
    /// `from` along `axis`, with the sharp part exact: the first crossing
    /// of `SHARP_SEARCH` pieces, narrowed down, and the normal there from
    /// differences of the exact density.
    fn exact_crossing(&self, from: [i32; 3], axis: usize, surface: &dyn ChunkSurface) -> Hermite {
        let at = |t: f32| {
            let mut p = from.map(|v| v as f32);
            p[axis] += t;
            p
        };
        let value = |t: f32| self.exact_value(at(t), surface);
        let (mut t0, mut v0) = (0.0, self.at(from[0], from[1], from[2]));
        let mut t1 = 1.0;
        let mut v1 = {
            let mut to = from;
            to[axis] += 1;
            self.at(to[0], to[1], to[2])
        };
        for k in 1..=SHARP_SEARCH {
            let t = k as f32 / SHARP_SEARCH as f32;
            let v = if k == SHARP_SEARCH { v1 } else { value(t) };
            if (v0 < 0.0) != (v < 0.0) {
                (t1, v1) = (t, v);
                break;
            }
            (t0, v0) = (t, v);
        }
        // Between t0 and t1 the sign changes: halve, then end on the
        // straight line between the two.
        for _ in 0..SHARP_REFINE {
            let t = 0.5 * (t0 + t1);
            let v = value(t);
            if (v0 < 0.0) == (v < 0.0) {
                (t0, v0) = (t, v);
            } else {
                (t1, v1) = (t, v);
            }
        }
        let t = if v0 == v1 { 0.5 * (t0 + t1) } else { t0 + (t1 - t0) * v0 / (v0 - v1) };
        let point = at(t);
        // Forward differences from the crossing: the density there is
        // (nearly) zero, so one sample per axis does.
        let h = SHARP_NORMAL_STEP;
        let here = self.exact_value(point, surface);
        let g: [f32; 3] = std::array::from_fn(|i| {
            let mut b = point;
            b[i] += h;
            self.exact_value(b, surface) - here
        });
        let normal = unit(g);
        let normal = if normal == [0.0; 3] { unit(self.gradient(point)) } else { normal };
        Hermite { point, normal }
    }

    /// Where the surface crosses the edge from chunk-local `from`, `step`
    /// metres along `axis` (whose ends are on either side of it), and its
    /// normal there: the first crossing between the full-detail samples
    /// along it, the normal from the full-detail cell beside it on the side
    /// of the cell asking (`back`: whether the edge lies on that cell's far
    /// side along the next and the one after the next axis).
    fn crossing(&self, from: [i32; 3], axis: usize, step: i32, back: [bool; 2]) -> Hermite {
        let (b, c) = ((axis + 1) % 3, (axis + 2) % 3);
        let mut a = from;
        let mut va = self.at(a[0], a[1], a[2]);
        for _ in 0..step {
            let mut next = a;
            next[axis] += 1;
            let vb = self.at(next[0], next[1], next[2]);
            if (va < 0.0) != (vb < 0.0) {
                let t = va / (va - vb);
                let mut point = a.map(|v| v as f32);
                point[axis] += t;
                let mut cell = a;
                if back[0] {
                    cell[b] -= 1;
                }
                if back[1] {
                    cell[c] -= 1;
                }
                return Hermite {
                    point,
                    normal: self.cell_normal(cell, point),
                };
            }
            (a, va) = (next, vb);
        }
        // The ends' signs differ, so a crossing lies between two samples;
        // only rounding gets here.
        let mut point = from.map(|v| v as f32);
        point[axis] += 0.5 * step as f32;
        let g = self.gradient(point);
        Hermite { point, normal: unit(g) }
    }

    /// The unit normal at chunk-local `point` of the field blended inside
    /// the full-detail cell with its first corner at `cell` (out of the
    /// ground): that cell's own slope, not its neighbours'.
    fn cell_normal(&self, cell: [i32; 3], point: [f32; 3]) -> [f32; 3] {
        let t: [f32; 3] = std::array::from_fn(|i| (point[i] - cell[i] as f32).clamp(0.0, 1.0));
        let v = |dx: i32, dy: i32, dz: i32| self.at(cell[0] + dx, cell[1] + dy, cell[2] + dz);
        let w = |d: i32, t: f32| if d == 1 { t } else { 1.0 - t };
        let mut g = [0.0f32; 3];
        for d1 in 0..2 {
            for d2 in 0..2 {
                g[0] += w(d1, t[1]) * w(d2, t[2]) * (v(1, d1, d2) - v(0, d1, d2));
                g[1] += w(d1, t[0]) * w(d2, t[2]) * (v(d1, 1, d2) - v(d1, 0, d2));
                g[2] += w(d1, t[0]) * w(d2, t[1]) * (v(d1, d2, 1) - v(d1, d2, 0));
            }
        }
        unit(g)
    }
}

/// The vertex of the cell from `low` to `high` for its `crossings`: the
/// best fit (see `solve_vertex`) where that lies in the cell; else the
/// middle of the crossings. Pulled back to the cell's side instead, it
/// would fold the triangles around it over where the planes meet outside
/// the cell (a crease of rock seen from a corner of it, say).
fn in_cell(crossings: &[Hermite], low: [f32; 3], high: [f32; 3]) -> [f32; 3] {
    let p = solve_vertex(crossings);
    if (0..3).all(|i| p[i] >= low[i] && p[i] <= high[i]) {
        return p;
    }
    let n = crossings.len().max(1) as f32;
    std::array::from_fn(|i| (crossings.iter().map(|h| h.point[i]).sum::<f32>() / n).clamp(low[i], high[i]))
}

fn unit(v: [f32; 3]) -> [f32; 3] {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if length < 1e-9 {
        [0.0; 3]
    } else {
        v.map(|c| c / length)
    }
}

/// The point nearest every crossing's plane (least squares), starting from
/// the middle of the crossings and moving only along directions the planes
/// pin down (see `TRUNCATION`).
pub fn solve_vertex(crossings: &[Hermite]) -> [f32; 3] {
    let count = crossings.len().max(1) as f32;
    let mut mass = [0.0f32; 3];
    for h in crossings {
        for i in 0..3 {
            mass[i] += h.point[i] / count;
        }
    }
    let mut ata = [[0.0f32; 3]; 3];
    let mut atb = [0.0f32; 3];
    for h in crossings {
        let n = h.normal;
        let d: f32 = (0..3).map(|i| n[i] * (h.point[i] - mass[i])).sum();
        for i in 0..3 {
            for j in 0..3 {
                ata[i][j] += n[i] * n[j];
            }
            atb[i] += n[i] * d;
        }
    }
    let x = truncated_solve(ata, atb);
    std::array::from_fn(|i| mass[i] + x[i])
}

/// `x` with `a x = b` for the symmetric `a`, through its eigenvectors,
/// ignoring the directions whose eigenvalue is below `TRUNCATION` of the
/// largest (a pseudo-inverse).
fn truncated_solve(mut a: [[f32; 3]; 3], b: [f32; 3]) -> [f32; 3] {
    // Jacobi rotations until `a` is diagonal; `v` gathers them, its
    // columns the eigenvectors.
    let mut v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0f32]];
    for _ in 0..8 {
        let off = a[0][1].abs() + a[0][2].abs() + a[1][2].abs();
        if off < 1e-9 {
            break;
        }
        for (p, q) in [(0, 1), (0, 2), (1, 2)] {
            if a[p][q].abs() < 1e-12 {
                continue;
            }
            let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
            let t = if theta == 0.0 {
                1.0
            } else {
                theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt())
            };
            let cos = 1.0 / (t * t + 1.0).sqrt();
            let sin = t * cos;
            for k in 0..3 {
                let (kp, kq) = (a[k][p], a[k][q]);
                a[k][p] = cos * kp - sin * kq;
                a[k][q] = sin * kp + cos * kq;
            }
            for k in 0..3 {
                let (pk, qk) = (a[p][k], a[q][k]);
                a[p][k] = cos * pk - sin * qk;
                a[q][k] = sin * pk + cos * qk;
            }
            for row in &mut v {
                let (kp, kq) = (row[p], row[q]);
                row[p] = cos * kp - sin * kq;
                row[q] = sin * kp + cos * kq;
            }
        }
    }
    let largest = a[0][0].max(a[1][1]).max(a[2][2]);
    let mut x = [0.0f32; 3];
    if largest <= 1e-9 {
        return x;
    }
    for i in 0..3 {
        let lambda = a[i][i];
        if lambda < TRUNCATION * largest {
            continue;
        }
        let column = [v[0][i], v[1][i], v[2][i]];
        let along: f32 = (0..3).map(|k| column[k] * b[k]).sum::<f32>() / lambda;
        for k in 0..3 {
            x[k] += along * column[k];
        }
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::{
        hills::{Hills, HillsSettings},
        source::{ChunkCoord, ChunkField, PlainSurface, SurfaceSample, TerrainGenerator, TerrainSource},
        CHUNK_SIZE_FLOAT,
    };
    use std::collections::HashMap;

    fn plane(point: [f32; 3], normal: [f32; 3]) -> Hermite {
        Hermite { point, normal: unit(normal) }
    }

    #[test]
    fn crossings_on_one_plane_give_a_point_on_it() {
        let n = [0.3, 1.0, -0.2];
        let on = |x: f32, z: f32| [x, 5.0 - (n[0] * x + n[2] * z) / n[1], z];
        let p = solve_vertex(&[plane(on(0.0, 0.0), n), plane(on(1.0, 0.0), n), plane(on(0.0, 1.0), n), plane(on(1.0, 1.0), n)]);
        let off: f32 = (0..3).map(|i| unit(n)[i] * (p[i] - on(0.0, 0.0)[i])).sum();
        assert!(off.abs() < 1e-4, "{p:?} is {off} m off the plane");
        // Along the plane it stays in the middle of the crossings.
        assert!((p[0] - 0.5).abs() < 1e-4 && (p[2] - 0.5).abs() < 1e-4, "{p:?}");
    }

    #[test]
    fn two_planes_put_the_vertex_on_their_ridge() {
        // A roof: rising from both sides to a ridge along z at x = 0.3,
        // y = 0.8.
        let (left, right) = ([-1.0, 1.0, 0.0], [1.0, 1.0, 0.0]);
        let crossings = [
            plane([0.0, 0.5, 0.0], left),
            plane([0.0, 0.5, 1.0], left),
            plane([1.0, 0.1, 0.0], right),
            plane([1.0, 0.1, 1.0], right),
        ];
        let p = solve_vertex(&crossings);
        assert!((p[0] - 0.3).abs() < 1e-3 && (p[1] - 0.8).abs() < 1e-3, "{p:?}");
        assert!((p[2] - 0.5).abs() < 1e-3, "{p:?}");
    }

    #[test]
    fn three_planes_put_the_vertex_on_their_corner() {
        let corner = [0.6, 0.4, 0.7];
        let crossings = [
            plane([corner[0], 0.0, 0.0], [1.0, 0.0, 0.0]),
            plane([corner[0], 1.0, 1.0], [1.0, 0.0, 0.0]),
            plane([0.0, corner[1], 0.0], [0.0, 1.0, 0.0]),
            plane([1.0, corner[1], 1.0], [0.0, 1.0, 0.0]),
            plane([0.0, 0.0, corner[2]], [0.0, 0.0, 1.0]),
        ];
        let p = solve_vertex(&crossings);
        for i in 0..3 {
            assert!((p[i] - corner[i]).abs() < 1e-3, "{p:?}");
        }
    }

    fn hills_chunk() -> (std::sync::Arc<Hills>, ChunkCoord) {
        let source = Hills::create(2, HillsSettings::default());
        let h = source.surface_height(64.0, 64.0);
        let coord = ChunkCoord::new(0, (h / CHUNK_SIZE_FLOAT).floor() as i32, 0);
        (source, coord)
    }

    /// Edges of a mesh each triangle has, counted both ways round.
    fn edges(indices: &[u32]) -> HashMap<(u32, u32), i32> {
        let mut edges = HashMap::new();
        for t in indices.chunks(3) {
            for k in 0..3 {
                *edges.entry((t[k], t[(k + 1) % 3])).or_insert(0) += 1;
            }
        }
        edges
    }

    #[test]
    fn the_mesh_is_closed_inside_the_chunk_on_every_level() {
        let (source, coord) = hills_chunk();
        let surface = source.chunk(coord, true);
        let field = DensityField { field: surface.field() };
        for lod in 0..4 {
            let step = 1 << lod;
            let (vertices, indices) = field.dual_contouring(step, &*surface).unwrap();
            let edges = edges(&indices);
            let inner = |p: [f32; 3]| (0..3).all(|i| i == 1 || (p[i] > step as f32 && p[i] < CHUNK_SIZE_FLOAT - step as f32));
            for (&(a, b), &n) in &edges {
                assert_eq!(n, 1, "level {lod}: edge {a}-{b} used {n} times the same way");
                let (pa, pb) = (vertices[a as usize].position, vertices[b as usize].position);
                if inner(pa) && inner(pb) {
                    assert!(edges.contains_key(&(b, a)), "level {lod}: open edge at {pa:?}");
                }
            }
        }
    }

    #[test]
    fn every_vertex_stays_in_its_cell_and_faces_match_the_normals() {
        let (source, coord) = hills_chunk();
        let surface = source.chunk(coord, true);
        let field = DensityField { field: surface.field() };
        for lod in 0..4 {
            let step = 1 << lod;
            let (vertices, indices) = field.dual_contouring(step, &*surface).unwrap();
            let reach = -(step as f32) - 1e-3..=CHUNK_SIZE_FLOAT + 2.0 * step as f32 + 1e-3;
            for v in &vertices {
                assert!(v.position.iter().all(|c| reach.contains(c)) || v.position[1] < 0.0 || v.position[1] > CHUNK_SIZE_FLOAT, "{:?}", v.position);
            }
            for t in indices.chunks(3) {
                let [a, b, c] = [t[0], t[1], t[2]].map(|i| vertices[i as usize]);
                let e1: [f32; 3] = std::array::from_fn(|k| b.position[k] - a.position[k]);
                let e2: [f32; 3] = std::array::from_fn(|k| c.position[k] - a.position[k]);
                let face = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
                let normal: [f32; 3] = std::array::from_fn(|k| a.normal[k] + b.normal[k] + c.normal[k]);
                let dot: f32 = (0..3).map(|k| face[k] * normal[k]).sum();
                assert!(dot >= -1e-4, "level {lod}: triangle at {:?} faces into the ground", a.position);
            }
        }
    }

    /// A field from `density(chunk-local x, y, z)` around chunk (0, 0, 0).
    fn field_of(density: impl Fn(f32, f32, f32) -> f32) -> ChunkField {
        let mut field = ChunkField::from_fn(ChunkCoord::new(0, 0, 0), |_, _| 64.0);
        field.fill_volume(|x, y, z, _| density(x as f32, y as f32, z as f32));
        field
    }

    #[test]
    fn a_block_is_no_rounder_than_with_surface_nets() {
        // An axis-aligned block, its corners between the samples. Its
        // faces are exact in the samples, but every corner cell holds the
        // kinks of three faces, which the samples blur: the corners only
        // come out sharp from exact crossings, not from these.
        let (low, high) = ([40.3, 50.6, 45.2], [80.7, 70.4, 90.5]);
        let field = field_of(|x, y, z| {
            let p = [x, y, z];
            (0..3).map(|i| (low[i] - p[i]).max(p[i] - high[i])).fold(f32::MIN, f32::max)
        });
        let surface = PlainSurface {
            field,
            surface: |_: &crate::terrain::source::SurfacePoint| SurfaceSample::plain([0.5; 3]),
        };
        let densities = DensityField { field: &surface.field };
        let (dc, _) = densities.dual_contouring(1, &surface).unwrap();
        let (sn, _) = densities.surface_nets(1, &surface).unwrap();
        let nearest = |vertices: &[TerrainVertex], c: [f32; 3]| {
            vertices
                .iter()
                .map(|v| (0..3).map(|i| (v.position[i] - c[i]).powi(2)).sum::<f32>().sqrt())
                .fold(f32::MAX, f32::min)
        };
        for corner in 0..8 {
            let c: [f32; 3] = std::array::from_fn(|i| if corner >> i & 1 == 1 { high[i] } else { low[i] });
            let (d, s) = (nearest(&dc, c), nearest(&sn, c));
            assert!(d <= s + 1e-3 && d < 0.75, "corner {c:?}: {d} m off (surface nets {s} m)");
        }
        // On the faces, away from the edges, the vertices lie on them.
        for v in &dc {
            let p = v.position;
            let inner = (0..3).filter(|&i| p[i] > low[i] + 1.5 && p[i] < high[i] - 1.5).count();
            if inner == 2 {
                let off = (0..3).map(|i| (low[i] - p[i]).max(p[i] - high[i])).fold(f32::MIN, f32::max);
                assert!(off.abs() < 1e-3, "{p:?} is {off} m off the face");
            }
        }
    }

    /// A block given as the field's sharp part (see
    /// `ChunkField::fill_volume_layered`), exactly.
    struct SharpBlock {
        field: ChunkField,
        low: [f32; 3],
        high: [f32; 3],
    }

    impl SharpBlock {
        fn density(&self, p: [f32; 3]) -> f32 {
            (0..3).map(|i| (self.low[i] - p[i]).max(p[i] - self.high[i])).fold(f32::MIN, f32::max)
        }
    }

    impl ChunkSurface for SharpBlock {
        fn field(&self) -> &ChunkField {
            &self.field
        }
        fn surface(&self, _: &crate::terrain::source::SurfacePoint) -> SurfaceSample {
            SurfaceSample::plain([0.5; 3])
        }
        fn sharp(&self, p: [f32; 3]) -> f32 {
            self.density(p)
        }
    }

    #[test]
    fn a_sharp_block_keeps_its_corners_and_edges() {
        let (low, high) = ([40.3, 50.6, 45.2], [80.7, 70.4, 90.5]);
        let mut block = SharpBlock {
            field: ChunkField::from_fn(ChunkCoord::new(0, 0, 0), |_, _| 64.0),
            low,
            high,
        };
        let mut field = block.field.clone();
        field.fill_volume_layered(|x, y, z, _| {
            let d = block.density([x as f32, y as f32, z as f32]);
            crate::terrain::source::VolumeSample { value: d, sharp: d, far: d }
        });
        block.field = field;
        let densities = DensityField { field: &block.field };
        let (vertices, indices) = densities.dual_contouring(1, &block).unwrap();
        let nearest = |c: [f32; 3]| {
            vertices
                .iter()
                .map(|v| (0..3).map(|i| (v.position[i] - c[i]).powi(2)).sum::<f32>().sqrt())
                .fold(f32::MAX, f32::min)
        };
        for corner in 0..8 {
            let c: [f32; 3] = std::array::from_fn(|i| if corner >> i & 1 == 1 { high[i] } else { low[i] });
            let d = nearest(c);
            assert!(d < 0.02, "corner {c:?}: nearest vertex {d} m away");
        }
        // Along an edge too, between the corners.
        let d = nearest([60.0, high[1], low[2]]);
        assert!(d < 0.6, "edge: {d} m");
        let mid_edge = vertices
            .iter()
            .filter(|v| (v.position[0] - 60.5).abs() <= 1.0)
            .map(|v| (v.position[1] - high[1]).abs().max((v.position[2] - low[2]).abs()))
            .fold(f32::MAX, f32::min);
        assert!(mid_edge < 0.02, "edge vertex {mid_edge} m off the edge");
        // Still closed and facing out.
        let edges = edges(&indices);
        for (&(a, b), _) in &edges {
            assert!(edges.contains_key(&(b, a)), "open edge at {:?}", vertices[a as usize].position);
        }
    }
}
