//! Shapes to look at a material on (the editor's material preview), each
//! standing on the origin with its base at y = 0 (the wind bends it about
//! that point), about a metre across.

use std::f32::consts::{PI, TAU};

use cgmath::{InnerSpace, Vector3};

use crate::core::model::{generated::MeshPart, Model};

/// A preview shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Shape {
    Sphere,
    Cube,
    Plane,
    Cylinder,
}

impl Shape {
    pub const ALL: [Shape; 4] = [Shape::Sphere, Shape::Cube, Shape::Plane, Shape::Cylinder];

    pub fn label(self) -> &'static str {
        match self {
            Shape::Sphere => "Kugel",
            Shape::Cube => "Würfel",
            Shape::Plane => "Ebene",
            Shape::Cylinder => "Zylinder",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Shape::Sphere => "sphere",
            Shape::Cube => "cube",
            Shape::Plane => "plane",
            Shape::Cylinder => "cylinder",
        }
    }

    pub fn from_name(name: &str) -> Option<Shape> {
        Shape::ALL.into_iter().find(|s| s.name() == name || s.label().eq_ignore_ascii_case(name))
    }

    /// The shape's mesh (one part, one material).
    pub fn mesh(self) -> MeshPart {
        match self {
            Shape::Sphere => sphere(0.5, 48, 24),
            Shape::Cube => cube(1.0),
            Shape::Plane => plane(2.0, 8),
            Shape::Cylinder => cylinder(0.35, 1.6, 40),
        }
    }

    /// The shape as a model (one material). Needs a GPU.
    pub fn model(self) -> Result<Model, String> {
        crate::core::model::generated::build_model(&[("shape", &self.mesh())])
    }

    /// Centre and radius to frame it.
    pub fn framing(self) -> ([f32; 3], f32) {
        match self {
            Shape::Sphere => ([0.0, 0.5, 0.0], 0.6),
            Shape::Cube => ([0.0, 0.5, 0.0], 0.9),
            Shape::Plane => ([0.0, 0.0, 0.0], 1.2),
            Shape::Cylinder => ([0.0, 0.8, 0.0], 0.95),
        }
    }
}

fn v(x: f32, y: f32, z: f32) -> Vector3<f32> {
    Vector3::new(x, y, z)
}

fn sphere(r: f32, segments: u32, rings: u32) -> MeshPart {
    let mut m = MeshPart::default();
    for j in 0..=rings {
        let t = j as f32 / rings as f32;
        let phi = t * PI;
        for i in 0..=segments {
            let s = i as f32 / segments as f32;
            let theta = s * TAU;
            let n = v(phi.sin() * theta.cos(), phi.cos(), phi.sin() * theta.sin());
            m.vertex(n * r + v(0.0, r, 0.0), n, [s * 2.0, t]);
        }
    }
    let row = segments + 1;
    for j in 0..rings {
        for i in 0..segments {
            let a = j * row + i;
            m.quad([a, a + 1, a + row + 1, a + row]);
        }
    }
    m
}

fn cube(size: f32) -> MeshPart {
    let mut m = MeshPart::default();
    let h = size * 0.5;
    // (normal, u axis, v axis) per face.
    let faces = [
        (v(1.0, 0.0, 0.0), v(0.0, 0.0, -1.0), v(0.0, 1.0, 0.0)),
        (v(-1.0, 0.0, 0.0), v(0.0, 0.0, 1.0), v(0.0, 1.0, 0.0)),
        (v(0.0, 1.0, 0.0), v(1.0, 0.0, 0.0), v(0.0, 0.0, -1.0)),
        (v(0.0, -1.0, 0.0), v(1.0, 0.0, 0.0), v(0.0, 0.0, 1.0)),
        (v(0.0, 0.0, 1.0), v(1.0, 0.0, 0.0), v(0.0, 1.0, 0.0)),
        (v(0.0, 0.0, -1.0), v(-1.0, 0.0, 0.0), v(0.0, 1.0, 0.0)),
    ];
    for (n, u, w) in faces {
        let c = n * h + v(0.0, h, 0.0);
        let corners = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
        let ids = corners.map(|(a, b)| m.vertex(c + u * (a * h) + w * (b * h), n, [(a + 1.0) * 0.5, 1.0 - (b + 1.0) * 0.5]));
        m.quad(ids);
    }
    m
}

fn plane(size: f32, cells: u32) -> MeshPart {
    let mut m = MeshPart::default();
    let up = v(0.0, 1.0, 0.0);
    for j in 0..=cells {
        for i in 0..=cells {
            let (s, t) = (i as f32 / cells as f32, j as f32 / cells as f32);
            m.vertex(v((s - 0.5) * size, 0.0, (0.5 - t) * size), up, [s * 2.0, 2.0 - t * 2.0]);
        }
    }
    let row = cells + 1;
    for j in 0..cells {
        for i in 0..cells {
            let a = j * row + i;
            m.quad([a, a + 1, a + row + 1, a + row]);
        }
    }
    m
}

fn cylinder(r: f32, height: f32, segments: u32) -> MeshPart {
    let mut m = MeshPart::default();
    let rows = 8;
    for j in 0..=rows {
        let t = j as f32 / rows as f32;
        for i in 0..=segments {
            let s = i as f32 / segments as f32;
            let a = s * TAU;
            let n = v(a.cos(), 0.0, -a.sin());
            m.vertex(v(n.x * r, t * height, n.z * r), n, [s * 2.0, 1.0 - t * 2.0]);
        }
    }
    let row = segments + 1;
    for j in 0..rows {
        for i in 0..segments {
            let a = j * row + i;
            m.quad([a, a + 1, a + row + 1, a + row]);
        }
    }
    // Top cap.
    let up = v(0.0, 1.0, 0.0);
    let centre = m.vertex(v(0.0, height, 0.0), up, [0.5, 0.5]);
    let first = m.positions.len() as u32;
    for i in 0..=segments {
        let a = i as f32 / segments as f32 * TAU;
        m.vertex(v(a.cos() * r, height, -a.sin() * r), up, [0.5 + a.cos() * 0.5, 0.5 - a.sin() * 0.5]);
    }
    for i in 0..segments {
        m.triangles.push([centre, first + i, first + i + 1]);
    }
    let _ = up.magnitude();
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_stand_on_the_origin_and_face_outwards() {
        for shape in Shape::ALL {
            let mesh = shape.mesh();
            let min_y = mesh.positions.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
            assert!(min_y.abs() < 1e-4, "{shape:?} starts at {min_y}");
            // Every triangle winds counter-clockwise seen along its normal.
            for t in &mesh.triangles {
                let [a, b, c] = t.map(|i| Vector3::from(mesh.positions[i as usize]));
                let face = (b - a).cross(c - a);
                let n = Vector3::from(mesh.normals[t[0] as usize]);
                if face.magnitude() > 1e-8 {
                    assert!(face.dot(n) > 0.0, "{shape:?}: a triangle faces inwards");
                }
            }
        }
    }
}
