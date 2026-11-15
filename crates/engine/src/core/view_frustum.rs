use cgmath::{InnerSpace, Matrix, Matrix4, Point3, Vector4};

use crate::terrain::ChunkBounds;

pub struct ViewFrustum {}

impl ViewFrustum {
    /// Whether any part of `bounds` may be visible through
    /// `view_projection`. Conservative: a box is only rejected when all its
    /// corners lie outside the same side of the view volume, so a box the
    /// camera is inside or looks into the middle of (no corner on screen)
    /// still counts as visible.
    pub fn is_bounds_in_frustum_matrix(view_projection: &Matrix4<f32>, bounds: ChunkBounds) -> bool {
        let (min, max) = (bounds.min, bounds.max);
        Self::is_box_visible(
            view_projection,
            Point3::new(min.0 as f32, min.1 as f32, min.2 as f32),
            Point3::new(max.0 as f32, max.1 as f32, max.2 as f32),
        )
    }

    /// The six planes of the view volume of `view_projection`, normalized,
    /// facing inwards: `dot(plane, (p, 1)) >= 0` inside.
    pub fn planes(view_projection: &Matrix4<f32>) -> [Vector4<f32>; 6] {
        let m = view_projection.transpose();
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

    /// As `is_bounds_in_frustum_matrix`, for a box `min`..`max` placed by
    /// `view_projection` (which may include a model transform).
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
