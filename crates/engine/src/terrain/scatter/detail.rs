//! The cells around the camera that detail outputs (grass, flowers,
//! pebbles) are planted in; see docs/scatter-graphs.md, step 4.
//!
//! The ground is cut into squares of `DETAIL_CELL` metres. A detail output
//! wants every cell whose square comes within its range of the camera, and
//! lets one go only once it is a cell's width further out, so walking along
//! the edge does not plant and drop the same cell over and over.

use super::{
    graph::{Placement, Rect},
    CHUNK_SIZE_FLOAT,
};

/// Edge of a detail cell, metres. A chunk column holds a whole number of
/// them.
pub const DETAIL_CELL: f32 = 32.0;

/// A cell by its square's index along x and z.
pub type CellKey = (i32, i32);

/// The ground a cell covers.
pub fn cell_rect((i, j): CellKey) -> Rect {
    let (x0, z0) = (i as f32 * DETAIL_CELL, j as f32 * DETAIL_CELL);
    Rect::new(x0, z0, x0 + DETAIL_CELL, z0 + DETAIL_CELL)
}

/// The chunk column a cell lies in.
pub fn cell_column((i, j): CellKey) -> (i32, i32) {
    let per = (CHUNK_SIZE_FLOAT / DETAIL_CELL) as i32;
    (i.div_euclid(per), j.div_euclid(per))
}

/// How far the nearest point of a cell's square is from `(x, z)`.
pub fn cell_distance(cell: CellKey, x: f32, z: f32) -> f32 {
    let r = cell_rect(cell);
    let dx = (r.x0 - x).max(0.0).max(x - r.x1);
    let dz = (r.z0 - z).max(0.0).max(z - r.z1);
    dx.hypot(dz)
}

/// The cells an output of range `distance` wants around `(x, z)`.
pub fn wanted_cells(x: f32, z: f32, distance: f32) -> Vec<CellKey> {
    let distance = distance.max(0.0);
    let cell = |v: f32| (v / DETAIL_CELL).floor() as i32;
    let (i0, i1) = (cell(x - distance), cell(x + distance));
    let (j0, j1) = (cell(z - distance), cell(z + distance));
    let mut cells = Vec::new();
    for j in j0..=j1 {
        for i in i0..=i1 {
            if cell_distance((i, j), x, z) <= distance {
                cells.push((i, j));
            }
        }
    }
    cells
}

/// Whether a planted cell stays for an output of range `distance` with the
/// camera at `(x, z)`.
pub fn keeps_cell(cell: CellKey, x: f32, z: f32, distance: f32) -> bool {
    cell_distance(cell, x, z) <= distance.max(0.0) + DETAIL_CELL
}

/// The placements standing within a chunk's heights, `floor..ceiling`.
pub fn in_band(placements: &[Placement], floor: f32, ceiling: f32) -> Vec<Placement> {
    placements
        .iter()
        .filter(|p| p.position[1] >= floor && p.position[1] < ceiling)
        .copied()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cells_wanted_reach_the_range_and_no_further() {
        let (x, z, range) = (10.0, -5.0, 60.0);
        let cells = wanted_cells(x, z, range);
        // About the circle's area in cells, plus its rim.
        assert!((12..=30).contains(&cells.len()), "{}", cells.len());
        assert!(cells.iter().all(|&c| cell_distance(c, x, z) <= range));
        // Every spot within range is in a wanted cell.
        for k in 0..64 {
            let a = k as f32 / 64.0 * std::f32::consts::TAU;
            let (px, pz) = (x + a.cos() * range * 0.99, z + a.sin() * range * 0.99);
            let c = ((px / DETAIL_CELL).floor() as i32, (pz / DETAIL_CELL).floor() as i32);
            assert!(cells.contains(&c), "{c:?}");
        }
        // The camera's own cell, whatever the range.
        assert_eq!(wanted_cells(x, z, 0.0), vec![(0, -1)]);
    }

    #[test]
    fn a_cell_is_let_go_a_cell_further_out() {
        let cell = (3, 0);
        // Its near edge at x = 96; the range 60.
        assert!(keeps_cell(cell, 36.0 - 30.0, 16.0, 60.0));
        assert!(keeps_cell(cell, 96.0 - 92.0, 16.0, 60.0));
        assert!(!keeps_cell(cell, 96.0 - 93.0, 16.0, 60.0));
        // Wanted again only within the range itself.
        assert!(!wanted_cells(96.0 - 61.0, 16.0, 60.0).contains(&cell));
        assert!(wanted_cells(96.0 - 59.0, 16.0, 60.0).contains(&cell));
    }

    #[test]
    fn cells_lie_in_their_column() {
        assert_eq!(cell_column((0, 0)), (0, 0));
        assert_eq!(cell_column((3, 4)), (0, 1));
        assert_eq!(cell_column((-1, -4)), (-1, -1));
        assert_eq!(cell_column((-5, 7)), (-2, 1));
        let r = cell_rect((-5, 7));
        assert_eq!((r.x0, r.z0, r.x1, r.z1), (-160.0, 224.0, -128.0, 256.0));
    }

    #[test]
    fn splitting_a_cell_by_chunk_height_loses_and_doubles_nothing() {
        let placements: Vec<Placement> = (0..200).map(|k| Placement::upright([k as f32 * 0.1, k as f32 * 2.0 - 150.0, 0.0], 0.0)).collect();
        let mut total = 0;
        for floor in (-256..256).step_by(128) {
            total += in_band(&placements, floor as f32, floor as f32 + 128.0).len();
        }
        assert_eq!(total, placements.len());
    }
}
