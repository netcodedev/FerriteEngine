use cgmath::{Matrix4, Vector4};

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
        let corners: [Vector4<f32>; 8] = std::array::from_fn(|i| {
            let x = if i & 1 == 0 { min.0 } else { max.0 };
            let y = if i & 2 == 0 { min.1 } else { max.1 };
            let z = if i & 4 == 0 { min.2 } else { max.2 };
            view_projection * Vector4::new(x as f32, y as f32, z as f32, 1.0)
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
    fn a_chunk_off_to_the_side_is_culled() {
        let vp = view_projection(Point3::new(16.0, 16.0, 200.0), Point3::new(16.0, 400.0, 200.0));
        assert!(!ViewFrustum::is_bounds_in_frustum_matrix(&vp, chunk()));
    }
}
