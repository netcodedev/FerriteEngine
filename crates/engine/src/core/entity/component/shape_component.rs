//! Simple shapes (cube, sphere, cylinder) in one colour, for blocking out
//! a level or standing in for a model that does not exist yet. They need
//! no assets: the mesh is generated, written to a temporary OBJ file and
//! loaded like any model. One metre across; the entity's scale sizes them.

use std::{fmt::Write as _, path::PathBuf};

use cgmath::{Matrix4, Point3, Vector3};
use russimp_ng::material::TextureType;
use serde::{Deserialize, Serialize};

use crate::core::{
    entity::Entity,
    headless,
    model::{Model, ModelBuilder},
    renderer::{pbr::Lighting, texture::Texture, weather::exposure},
    scene::{scene_file::ComponentData, Scene},
};

use super::{Component, Property, PropertyValue};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeKind {
    Cube,
    Sphere,
    Cylinder,
}

impl ShapeKind {
    pub const ALL: [ShapeKind; 3] = [ShapeKind::Cube, ShapeKind::Sphere, ShapeKind::Cylinder];

    pub fn name(self) -> &'static str {
        match self {
            ShapeKind::Cube => "cube",
            ShapeKind::Sphere => "sphere",
            ShapeKind::Cylinder => "cylinder",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name().eq_ignore_ascii_case(name))
    }

    /// The mesh as OBJ text: positions, normals and texture coordinates,
    /// centred on the origin, one metre across.
    pub fn obj(self) -> String {
        let mesh = match self {
            ShapeKind::Cube => cube(),
            ShapeKind::Sphere => sphere(12, 24),
            ShapeKind::Cylinder => cylinder(24),
        };
        mesh.to_obj()
    }
}

#[derive(Default)]
struct Mesh {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    triangles: Vec<[usize; 3]>,
}

impl Mesh {
    fn vertex(&mut self, position: [f32; 3], normal: [f32; 3], uv: [f32; 2]) -> usize {
        self.positions.push(position);
        self.normals.push(normal);
        self.uvs.push(uv);
        self.positions.len() - 1
    }

    fn quad(&mut self, a: usize, b: usize, c: usize, d: usize) {
        self.triangles.push([a, b, c]);
        self.triangles.push([a, c, d]);
    }

    fn to_obj(&self) -> String {
        let mut text = String::new();
        for [x, y, z] in &self.positions {
            let _ = writeln!(text, "v {x} {y} {z}");
        }
        for [x, y, z] in &self.normals {
            let _ = writeln!(text, "vn {x} {y} {z}");
        }
        for [u, v] in &self.uvs {
            let _ = writeln!(text, "vt {u} {v}");
        }
        // OBJ indices start at 1; every vertex has its own normal and uv.
        for [a, b, c] in &self.triangles {
            let (a, b, c) = (a + 1, b + 1, c + 1);
            let _ = writeln!(text, "f {a}/{a}/{a} {b}/{b}/{b} {c}/{c}/{c}");
        }
        text
    }
}

fn cube() -> Mesh {
    let mut mesh = Mesh::default();
    // Each face: normal, then two axes spanning it (counter-clockwise seen
    // from outside).
    let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
        ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
        ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
        ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
    ];
    for (n, u, v) in faces {
        let corner = |su: f32, sv: f32| {
            std::array::from_fn(|i| 0.5 * n[i] + 0.5 * su * u[i] + 0.5 * sv * v[i])
        };
        let a = mesh.vertex(corner(-1.0, -1.0), n, [0.0, 0.0]);
        let b = mesh.vertex(corner(1.0, -1.0), n, [1.0, 0.0]);
        let c = mesh.vertex(corner(1.0, 1.0), n, [1.0, 1.0]);
        let d = mesh.vertex(corner(-1.0, 1.0), n, [0.0, 1.0]);
        mesh.quad(a, b, c, d);
    }
    mesh
}

fn sphere(rings: usize, segments: usize) -> Mesh {
    let mut mesh = Mesh::default();
    for ring in 0..=rings {
        let v = ring as f32 / rings as f32;
        let polar = v * std::f32::consts::PI;
        for segment in 0..=segments {
            let u = segment as f32 / segments as f32;
            let azimuth = u * std::f32::consts::TAU;
            let n = [
                polar.sin() * azimuth.cos(),
                polar.cos(),
                -polar.sin() * azimuth.sin(),
            ];
            mesh.vertex([n[0] * 0.5, n[1] * 0.5, n[2] * 0.5], n, [u, 1.0 - v]);
        }
    }
    let row = segments + 1;
    for ring in 0..rings {
        for segment in 0..segments {
            let a = ring * row + segment;
            let b = a + row;
            mesh.quad(a, b, b + 1, a + 1);
        }
    }
    mesh
}

fn cylinder(segments: usize) -> Mesh {
    let mut mesh = Mesh::default();
    let point = |i: usize| {
        let angle = i as f32 / segments as f32 * std::f32::consts::TAU;
        (angle.cos(), -angle.sin())
    };
    // The side.
    for i in 0..segments {
        let (x0, z0) = point(i);
        let (x1, z1) = point(i + 1);
        let u0 = i as f32 / segments as f32;
        let u1 = (i + 1) as f32 / segments as f32;
        let a = mesh.vertex([x0 * 0.5, -0.5, z0 * 0.5], [x0, 0.0, z0], [u0, 0.0]);
        let b = mesh.vertex([x1 * 0.5, -0.5, z1 * 0.5], [x1, 0.0, z1], [u1, 0.0]);
        let c = mesh.vertex([x1 * 0.5, 0.5, z1 * 0.5], [x1, 0.0, z1], [u1, 1.0]);
        let d = mesh.vertex([x0 * 0.5, 0.5, z0 * 0.5], [x0, 0.0, z0], [u0, 1.0]);
        mesh.quad(a, b, c, d);
    }
    // The caps, as fans around their centres.
    for (y, ny) in [(0.5f32, 1.0f32), (-0.5, -1.0)] {
        let centre = mesh.vertex([0.0, y, 0.0], [0.0, ny, 0.0], [0.5, 0.5]);
        for i in 0..segments {
            let (x0, z0) = point(i);
            let (x1, z1) = point(i + 1);
            let a = mesh.vertex([x0 * 0.5, y, z0 * 0.5], [0.0, ny, 0.0], [0.5 + x0 * 0.5, 0.5 + z0 * 0.5]);
            let b = mesh.vertex([x1 * 0.5, y, z1 * 0.5], [0.0, ny, 0.0], [0.5 + x1 * 0.5, 0.5 + z1 * 0.5]);
            if ny > 0.0 {
                mesh.triangles.push([centre, a, b]);
            } else {
                mesh.triangles.push([centre, b, a]);
            }
        }
    }
    mesh
}

/// Where the generated OBJ of a shape is written.
fn obj_path(kind: ShapeKind) -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join(format!("ferrite-shapes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(format!("{}.obj", kind.name()));
    if !path.is_file() {
        std::fs::write(&path, kind.obj()).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(path)
}

fn color_byte(channel: f32) -> u8 {
    (channel.clamp(0.0, 1.0) * 255.0).round() as u8
}

pub struct ShapeComponent {
    kind: ShapeKind,
    /// sRGB, each channel 0 to 1.
    color: [f32; 3],
    /// Not made without a GPU (the dedicated server).
    model: Option<Model>,
}

impl ShapeComponent {
    pub fn new(kind: ShapeKind, color: [f32; 3]) -> Result<Self, String> {
        let mut shape = ShapeComponent {
            kind,
            color,
            model: None,
        };
        shape.rebuild()?;
        Ok(shape)
    }

    pub fn kind(&self) -> ShapeKind {
        self.kind
    }

    pub fn color(&self) -> [f32; 3] {
        self.color
    }

    pub fn set_kind(&mut self, kind: ShapeKind) {
        if kind != self.kind {
            self.kind = kind;
            if let Err(e) = self.rebuild() {
                log::warn!("shape: {e}");
            }
        }
    }

    pub fn set_color(&mut self, color: [f32; 3]) {
        self.color = color.map(|c| c.clamp(0.0, 1.0));
        self.apply_color();
    }

    fn rebuild(&mut self) -> Result<(), String> {
        if headless::is_headless() {
            return Ok(());
        }
        let path = obj_path(self.kind)?;
        let mut model = ModelBuilder::from_file(&path.to_string_lossy())
            .map_err(|e| format!("could not build a {}: {e}", self.kind.name()))?
            .with_scale(1.0)
            .build();
        model.init();
        self.model = Some(model);
        self.apply_color();
        Ok(())
    }

    fn apply_color(&mut self) {
        let Some(model) = &mut self.model else {
            return;
        };
        let [r, g, b] = self.color.map(color_byte);
        for material in 0..model.material_count().max(1) {
            let texture = Texture::new();
            texture.load_material_map(1, 1, vec![r, g, b, 255], true);
            model.set_material_texture(material, TextureType::Diffuse, texture);
        }
    }
}

impl Component for ShapeComponent {
    fn update(&mut self, _: &mut Scene, _: &mut Entity, _: f64) {}

    fn render(
        &self,
        scene: &Scene,
        _: &Entity,
        view_projection: &Matrix4<f32>,
        parent_transform: &Matrix4<f32>,
    ) {
        // The ground map holds the terrain alone.
        if exposure::in_ground_pass() {
            return;
        }
        if let (Some(model), Some(lighting)) = (&self.model, Lighting::from_scene(scene)) {
            model.render(&lighting, parent_transform, view_projection);
        }
    }

    fn handle_event(&mut self, _: &mut glfw::Glfw, _: &mut glfw::Window, _: &glfw::WindowEvent) {}

    fn raycast(
        &self,
        transform: &Matrix4<f32>,
        origin: Point3<f32>,
        dir: Vector3<f32>,
    ) -> Option<f32> {
        self.model.as_ref()?.raycast(transform, origin, dir)
    }

    fn display_name(&self) -> String {
        "Shape".into()
    }

    fn to_data(&self) -> Option<ComponentData> {
        Some(ComponentData::Shape {
            kind: self.kind,
            color: self.color,
        })
    }

    fn properties(&self) -> Vec<Property> {
        let [r, g, b] = self.color;
        vec![
            Property::choice(
                "shape",
                self.kind.name().to_string(),
                ShapeKind::ALL.iter().map(|k| k.name().to_string()).collect(),
            ),
            Property::ranged("color r", r, 0.0, 1.0),
            Property::ranged("color g", g, 0.0, 1.0),
            Property::ranged("color b", b, 0.0, 1.0),
        ]
    }

    fn set_property(&mut self, name: &str, value: PropertyValue) {
        let mut color = self.color;
        match name {
            "shape" => {
                if let Some(kind) = value.as_text().and_then(ShapeKind::from_name) {
                    self.set_kind(kind);
                }
                return;
            }
            "color r" => color[0] = value.as_f32().unwrap_or(color[0]),
            "color g" => color[1] = value.as_f32().unwrap_or(color[1]),
            "color b" => color[2] = value.as_f32().unwrap_or(color[2]),
            _ => return,
        }
        self.set_color(color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(obj: &str) -> (usize, usize, usize, Vec<[f32; 3]>) {
        let mut positions = Vec::new();
        let (mut normals, mut uvs, mut faces) = (0, 0, 0);
        for line in obj.lines() {
            let mut parts = line.split_whitespace();
            match parts.next() {
                Some("v") => {
                    let p: Vec<f32> = parts.map(|n| n.parse().unwrap()).collect();
                    positions.push([p[0], p[1], p[2]]);
                }
                Some("vn") => normals += 1,
                Some("vt") => uvs += 1,
                Some("f") => {
                    faces += 1;
                    for corner in parts {
                        let index: usize = corner.split('/').next().unwrap().parse().unwrap();
                        assert!(index >= 1 && index <= positions.len(), "face index {index}");
                    }
                }
                _ => {}
            }
        }
        assert_eq!(normals, positions.len());
        assert_eq!(uvs, positions.len());
        (positions.len(), normals, faces, positions)
    }

    #[test]
    fn every_shape_is_a_valid_obj_within_one_metre() {
        for kind in ShapeKind::ALL {
            let (vertices, _, faces, positions) = parse(&kind.obj());
            assert!(vertices > 0 && faces > 0, "{kind:?}");
            for p in positions {
                for c in p {
                    assert!(c.abs() <= 0.5 + 1e-5, "{kind:?} reaches {c}");
                }
            }
        }
    }

    #[test]
    fn a_cube_has_six_faces_of_two_triangles() {
        let (vertices, _, faces, _) = parse(&ShapeKind::Cube.obj());
        assert_eq!(vertices, 24);
        assert_eq!(faces, 12);
    }

    #[test]
    fn cube_triangles_face_outwards() {
        let mesh = cube();
        for [a, b, c] in mesh.triangles {
            let (pa, pb, pc) = (mesh.positions[a], mesh.positions[b], mesh.positions[c]);
            let e1 = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
            let e2 = [pc[0] - pa[0], pc[1] - pa[1], pc[2] - pa[2]];
            let cross = [
                e1[1] * e2[2] - e1[2] * e2[1],
                e1[2] * e2[0] - e1[0] * e2[2],
                e1[0] * e2[1] - e1[1] * e2[0],
            ];
            let n = mesh.normals[a];
            let dot = cross[0] * n[0] + cross[1] * n[1] + cross[2] * n[2];
            assert!(dot > 0.0, "triangle {a},{b},{c} faces inwards");
        }
    }

    #[test]
    fn shape_names_round_trip() {
        for kind in ShapeKind::ALL {
            assert_eq!(ShapeKind::from_name(kind.name()), Some(kind));
        }
        assert_eq!(ShapeKind::from_name("Sphere"), Some(ShapeKind::Sphere));
        assert_eq!(ShapeKind::from_name("teapot"), None);
    }

    #[test]
    fn colours_become_bytes_and_are_clamped() {
        assert_eq!(color_byte(0.0), 0);
        assert_eq!(color_byte(1.0), 255);
        assert_eq!(color_byte(2.0), 255);
        assert_eq!(color_byte(-1.0), 0);
        assert_eq!(color_byte(0.5), 128);
    }
}
