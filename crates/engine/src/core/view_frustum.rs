use std::cell::RefCell;

use cgmath::{InnerSpace, Matrix, Matrix4, Point3, SquareMatrix, Vector4};

use crate::terrain::ChunkBounds;

/// What is culled, counted apart (see `ViewFrustum::counts`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Culled {
    /// Terrain chunks.
    Chunk,
    /// Groups of scattered models (trees, rocks: those standing in the
    /// same 16 m square).
    ScatterCell,
    /// Models (entities with a model).
    Model,
}

impl Culled {
    pub const ALL: [Culled; 3] = [Culled::Chunk, Culled::ScatterCell, Culled::Model];
}

/// How many of a kind were drawn and how many were looked at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CullCount {
    pub drawn: u32,
    pub total: u32,
}

/// The culling of the camera's view (see `ViewFrustum::begin_view`).
#[derive(Default)]
struct ViewCulling {
    /// The camera's view and projection this frame, while its passes run.
    view: Option<Matrix4<f32>>,
    /// Whether culling is to stay as it was (`set_frozen`), and the view
    /// it stays at.
    freeze: bool,
    frozen: Option<Matrix4<f32>>,
    /// Whether the pass that counts (the shading pass) runs, what it
    /// counted so far, and what the last one counted.
    counting: bool,
    counts: [CullCount; 3],
    last: [CullCount; 3],
}

thread_local! {
    static VIEW: RefCell<ViewCulling> = RefCell::new(ViewCulling::default());
}

pub struct ViewFrustum {}

impl ViewFrustum {
    /// The camera's passes this frame begin: they cull against
    /// `view_projection`, or against the view frozen with `set_frozen`.
    /// Other views (shadow maps) cull against their own.
    pub fn begin_view(view_projection: Matrix4<f32>) {
        VIEW.with(|v| {
            let mut v = v.borrow_mut();
            v.view = Some(view_projection);
            if v.freeze && v.frozen.is_none() {
                v.frozen = Some(view_projection);
            }
        });
    }

    /// The camera's passes are done.
    pub fn end_view() {
        VIEW.with(|v| v.borrow_mut().view = None);
    }

    /// Keep culling the camera's view as it is now (`true`) while the
    /// camera moves on: what lies outside it is left out, as it would be
    /// from there, to see what culling leaves out. `false`: follow the
    /// camera again.
    pub fn set_frozen(freeze: bool) {
        VIEW.with(|v| {
            let mut v = v.borrow_mut();
            v.freeze = freeze;
            if !freeze {
                v.frozen = None;
            }
        });
    }

    /// The view culling is frozen at, if it is.
    pub fn frozen() -> Option<Matrix4<f32>> {
        VIEW.with(|v| v.borrow().frozen)
    }

    /// The corners of the frozen view (near, then at `reach` metres),
    /// four each, for drawing it.
    pub fn frozen_corners(reach: f32) -> Option<[Point3<f32>; 8]> {
        let inverse = Self::frozen()?.invert()?;
        let at = |x: f32, y: f32, z: f32| {
            let p = inverse * Vector4::new(x, y, z, 1.0);
            Point3::new(p.x / p.w, p.y / p.w, p.z / p.w)
        };
        let near: [Point3<f32>; 4] = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(x, y)| at(x, y, -1.0));
        let deep: [Point3<f32>; 4] = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(x, y)| at(x, y, 1.0));
        let mut corners = [near[0]; 8];
        for i in 0..4 {
            corners[i] = near[i];
            let direction = (deep[i] - near[i]).normalize();
            corners[4 + i] = near[i] + direction * reach;
        }
        Some(corners)
    }

    /// Count what the shading pass culls (from `begin_counting` to
    /// `end_counting`); the depth passes cull the same.
    pub fn begin_counting() {
        VIEW.with(|v| {
            let mut v = v.borrow_mut();
            v.counting = true;
            v.counts = Default::default();
        });
    }

    pub fn end_counting() {
        VIEW.with(|v| {
            let mut v = v.borrow_mut();
            v.counting = false;
            v.last = v.counts;
        });
    }

    /// One of `kind` was drawn (`visible`) or culled, if counting.
    pub fn count(kind: Culled, visible: bool) {
        Self::count_many(kind, 1, if visible { 1 } else { 0 });
    }

    /// `total` of `kind` were looked at, `drawn` of them drawn, if
    /// counting.
    pub fn count_many(kind: Culled, total: u32, drawn: u32) {
        VIEW.with(|v| {
            let mut v = v.borrow_mut();
            if v.counting {
                let count = &mut v.counts[kind as usize];
                count.total += total;
                count.drawn += drawn;
            }
        });
    }

    /// What the last shading pass drew and culled, by kind (in
    /// `Culled::ALL`'s order).
    pub fn counts() -> [CullCount; 3] {
        VIEW.with(|v| v.borrow().last)
    }

    /// The view to cull against when drawing with `view_projection`: the
    /// frozen one in the camera's passes (see `set_frozen`), else itself.
    pub fn cull_view(view_projection: &Matrix4<f32>) -> Matrix4<f32> {
        VIEW.with(|v| {
            let v = v.borrow();
            match (v.view, v.frozen) {
                (Some(view), Some(frozen)) if view == *view_projection => frozen,
                _ => *view_projection,
            }
        })
    }

    /// Whether any part of `bounds` may be visible through
    /// `view_projection`. Conservative: a box is only rejected when all its
    /// corners lie outside the same side of the view volume, so a box the
    /// camera is inside or looks into the middle of (no corner on screen)
    /// still counts as visible.
    pub fn is_bounds_in_frustum_matrix(view_projection: &Matrix4<f32>, bounds: ChunkBounds) -> bool {
        let (min, max) = (bounds.min, bounds.max);
        Self::is_box_visible(
            &Self::cull_view(view_projection),
            Point3::new(min.0 as f32, min.1 as f32, min.2 as f32),
            Point3::new(max.0 as f32, max.1 as f32, max.2 as f32),
        )
    }

    /// The six planes of the view volume of `view_projection`, normalized,
    /// facing inwards: `dot(plane, (p, 1)) >= 0` inside.
    pub fn planes(view_projection: &Matrix4<f32>) -> [Vector4<f32>; 6] {
        let m = Self::cull_view(view_projection).transpose();
        let (r0, r1, r2, r3) = (m.x, m.y, m.z, m.w);
        [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r3 + r2, r3 - r2].map(|plane| {
            let length = plane.truncate().magnitude();
            if length > 0.0 {
                plane / length
            } else {
                plane
            }
        })
    }

    /// Whether a sphere is (at least partly) inside `planes` (see
    /// `planes`). Conservative near the volume's corners.
    pub fn is_sphere_visible(planes: &[Vector4<f32>; 6], center: Point3<f32>, radius: f32) -> bool {
        planes.iter().all(|plane| {
            plane.x * center.x + plane.y * center.y + plane.z * center.z + plane.w >= -radius
        })
    }

    /// As `is_bounds_in_frustum_matrix`, for a box `min`..`max` of a model
    /// placed by `model` in the world drawn with `view_projection`.
    pub fn is_model_box_visible(
        view_projection: &Matrix4<f32>,
        model: &Matrix4<f32>,
        min: Point3<f32>,
        max: Point3<f32>,
    ) -> bool {
        Self::is_box_visible(&(Self::cull_view(view_projection) * model), min, max)
    }

    /// As `is_bounds_in_frustum_matrix`, for a box `min`..`max` placed by
    /// `view_projection` (which may include a model transform; not frozen,
    /// see `is_model_box_visible`).
    pub fn is_box_visible(view_projection: &Matrix4<f32>, min: Point3<f32>, max: Point3<f32>) -> bool {
        let corners: [Vector4<f32>; 8] = std::array::from_fn(|i| {
            let x = if i & 1 == 0 { min.x } else { max.x };
            let y = if i & 2 == 0 { min.y } else { max.y };
            let z = if i & 4 == 0 { min.z } else { max.z };
            view_projection * Vector4::new(x, y, z, 1.0)
        });
        // The six clip planes: -w <= x, y, z <= w.
        let outside: [fn(&Vector4<f32>) -> bool; 6] = [
            |p| p.x < -p.w,
            |p| p.x > p.w,
            |p| p.y < -p.w,
            |p| p.y > p.w,
            |p| p.z < -p.w,
            |p| p.z > p.w,
        ];
        !outside
            .iter()
            .any(|is_outside| corners.iter().all(is_outside))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgmath::{perspective, Deg, Point3, Vector3};

    fn view_projection(eye: Point3<f32>, target: Point3<f32>) -> Matrix4<f32> {
        perspective(Deg(30.0), 1.0, 0.1, 1000.0)
            * Matrix4::look_at_rh(eye, target, Vector3::unit_z())
    }

    fn chunk() -> ChunkBounds {
        ChunkBounds {
            min: (0, 0, 0),
            max: (32, 32, 32),
        }
    }

    #[test]
    fn a_chunk_seen_from_close_above_its_middle_is_visible() {
        // All eight corners are off screen, but the chunk fills the view.
        let vp = view_projection(Point3::new(16.0, 33.0, 16.0), Point3::new(16.0, 0.0, 16.0));
        assert!(ViewFrustum::is_bounds_in_frustum_matrix(&vp, chunk()));
    }

    #[test]
    fn a_chunk_the_camera_is_inside_is_visible() {
        let vp = view_projection(Point3::new(16.0, 16.0, 16.0), Point3::new(40.0, 16.0, 16.0));
        assert!(ViewFrustum::is_bounds_in_frustum_matrix(&vp, chunk()));
    }

    #[test]
    fn a_chunk_behind_the_camera_is_culled() {
        let vp = view_projection(Point3::new(100.0, 16.0, 16.0), Point3::new(200.0, 16.0, 16.0));
        assert!(!ViewFrustum::is_bounds_in_frustum_matrix(&vp, chunk()));
    }

    #[test]
    fn spheres_are_culled_by_the_planes() {
        let vp = view_projection(Point3::new(0.0, 0.0, 0.0), Point3::new(100.0, 0.0, 0.0));
        let planes = ViewFrustum::planes(&vp);
        assert!(ViewFrustum::is_sphere_visible(&planes, Point3::new(50.0, 0.0, 0.0), 1.0));
        assert!(!ViewFrustum::is_sphere_visible(&planes, Point3::new(-50.0, 0.0, 0.0), 1.0));
        assert!(!ViewFrustum::is_sphere_visible(&planes, Point3::new(50.0, 0.0, 80.0), 5.0));
        // Off to the side, but big enough to reach into the view.
        assert!(ViewFrustum::is_sphere_visible(&planes, Point3::new(50.0, 0.0, 30.0), 25.0));
        assert!(!ViewFrustum::is_sphere_visible(&planes, Point3::new(2000.0, 0.0, 0.0), 1.0));
    }

    #[test]
    fn a_chunk_off_to_the_side_is_culled() {
        let vp = view_projection(Point3::new(16.0, 16.0, 200.0), Point3::new(16.0, 400.0, 200.0));
        assert!(!ViewFrustum::is_bounds_in_frustum_matrix(&vp, chunk()));
    }
}
