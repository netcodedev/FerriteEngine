use cgmath::{InnerSpace, Vector4};

use crate::terrain::{ChunkBounds, CHUNK_SIZE};

use super::camera::{Camera, Projection};

pub struct ViewFrustum {}

impl ViewFrustum {
    pub fn is_bounds_in_frustum(
        projection: &Projection,
        camera: &Camera,
        bounds: ChunkBounds,
    ) -> bool {
        let mut result = false;

        // check if bounds are close to camera
        let distance = (camera.position - bounds.center()).magnitude();
        if distance < CHUNK_SIZE as f32 * 0.75 {
            return true;
        }

        let view_projection = projection.calc_matrix() * camera.calc_matrix();
        let clip: [Vector4<f32>; 8] = [
            Vector4::new(
                bounds.min.0 as f32,
                bounds.min.1 as f32,
                bounds.min.2 as f32,
                1.0,
            ),
            Vector4::new(
                bounds.min.0 as f32,
                bounds.min.1 as f32,
                bounds.max.2 as f32,
                1.0,
            ),
            Vector4::new(
                bounds.min.0 as f32,
                bounds.max.1 as f32,
                bounds.min.2 as f32,
                1.0,
            ),
            Vector4::new(
                bounds.min.0 as f32,
                bounds.max.1 as f32,
                bounds.max.2 as f32,
                1.0,
            ),
            Vector4::new(
                bounds.max.0 as f32,
                bounds.min.1 as f32,
                bounds.min.2 as f32,
                1.0,
            ),
            Vector4::new(
                bounds.max.0 as f32,
                bounds.min.1 as f32,
                bounds.max.2 as f32,
                1.0,
            ),
            Vector4::new(
                bounds.max.0 as f32,
                bounds.max.1 as f32,
                bounds.min.2 as f32,
                1.0,
            ),
            Vector4::new(
                bounds.max.0 as f32,
                bounds.max.1 as f32,
                bounds.max.2 as f32,
                1.0,
            ),
        ];

        for point in clip {
            let point = view_projection * point;
            if point.x <= point.w
                && point.x >= -point.w
                && point.y <= point.w
                && point.y >= -point.w
                && point.z <= point.w
                && point.z >= -point.w
            {
                result = true;
                break;
            }
        }

        result
    }
}
