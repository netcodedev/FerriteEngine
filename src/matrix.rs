use cgmath::{Matrix4, perspective, Deg, Point3, Vector3};
use glfw::Glfw;

pub fn create_transformation_matrices(glfw: &Glfw) -> (Matrix4<f32>, Matrix4<f32>) {
    let view: Matrix4<f32> = Matrix4::look_at_rh(
        Point3::new(20.0, 20.0, 30.0), // Camera position
        Point3::new(0.0, 0.0, 0.0), // Target point
        Vector3::new(0.0, 1.0, 0.0), // Up vector
    );

    let projection: Matrix4<f32> = perspective(Deg(45.0), 800.0 / 600.0, 0.1, 100.0);

    (view, projection)
}