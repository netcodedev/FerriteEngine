//! Meshes made in code (trees, bridges) turned into models: written to a
//! temporary OBJ file with one material per part and loaded like any model,
//! so they are drawn, shadowed and picked like one.

use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use cgmath::Vector3;

use super::Model;
#[cfg(test)]
use super::ModelBuilder;

/// A triangle mesh: positions, normals, texture coordinates. Triangles wind
/// counter-clockwise seen from where they face.
#[derive(Default, Debug, Clone)]
pub struct MeshPart {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub triangles: Vec<[u32; 3]>,
}

impl MeshPart {
    pub fn vertex(&mut self, p: Vector3<f32>, n: Vector3<f32>, uv: [f32; 2]) -> u32 {
        self.positions.push(p.into());
        self.normals.push(n.into());
        self.uvs.push(uv);
        (self.positions.len() - 1) as u32
    }

    /// A quad `a b c d`, counter-clockwise seen from its front.
    pub fn quad(&mut self, corners: [u32; 4]) {
        let [a, b, c, d] = corners;
        self.triangles.extend([[a, b, c], [a, c, d]]);
    }

    /// A quad seen from both sides (foliage): each side faces its own way
    /// for the winding, but both carry the corners' normals, so a leaf is
    /// lit the same from either side.
    pub fn two_sided(&mut self, corners: [u32; 4]) {
        let [a, b, c, d] = corners;
        self.triangles.extend([[a, b, c], [a, c, d], [a, c, b], [a, d, c]]);
    }

    pub fn is_empty(&self) -> bool {
        self.triangles.is_empty()
    }
}

/// A fresh temporary file name for an OBJ (its MTL beside it).
fn obj_path() -> Result<PathBuf, String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!("ferrite-generated-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir.join(format!("mesh-{}.obj", NEXT.fetch_add(1, Ordering::Relaxed))))
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// `parts` as an OBJ file, each its own object and material of its name
/// (empty parts left out). Returns its path; the MTL lies beside it.
pub fn write_obj(parts: &[(&str, &MeshPart)]) -> Result<PathBuf, String> {
    let path = obj_path()?;
    let mtl = path.with_extension("mtl");
    let mtl_name = mtl.file_name().unwrap().to_string_lossy().to_string();
    let mut text = format!("mtllib {mtl_name}\n");
    let mut materials = String::new();
    let mut base = 1;
    for (name, part) in parts {
        if part.is_empty() {
            continue;
        }
        let _ = writeln!(materials, "newmtl {name}\nKd 1 1 1\n");
        let _ = writeln!(text, "o {name}\nusemtl {name}");
        for [x, y, z] in &part.positions {
            let _ = writeln!(text, "v {x:.4} {y:.4} {z:.4}");
        }
        for [x, y, z] in &part.normals {
            let _ = writeln!(text, "vn {x:.4} {y:.4} {z:.4}");
        }
        for [u, v] in &part.uvs {
            let _ = writeln!(text, "vt {u:.4} {v:.4}");
        }
        for t in &part.triangles {
            let [a, b, c] = t.map(|i| i as usize + base);
            let _ = writeln!(text, "f {a}/{a}/{a} {b}/{b}/{b} {c}/{c}/{c}");
        }
        base += part.positions.len();
    }
    write(&path, &text)?;
    write(&mtl, &materials)?;
    Ok(path)
}

/// `parts` as a model, its meshes and materials named as the parts (empty
/// parts left out), drawn, shadowed and picked like a loaded one. Made in
/// memory as assimp would read `write_obj`'s file (one object, mesh and
/// material per part, UVs flipped as `FlipUVs` does), without writing or
/// parsing one: that took tens of milliseconds per town block, on the
/// frame that showed it. Needs a GPU.
pub fn build_model(parts: &[(&str, &MeshPart)]) -> Result<Model, String> {
    let scene = scene_of(parts);
    // Nothing to find next to it: maps are given in code.
    let mut model = Model::from_scene(scene, "generated".into(), String::from("\0generated"), (0.0, 0.0, 0.0));
    model.scale = 1.0;
    model.init();
    Ok(model)
}

/// The assimp scene `build_model` makes a model of.
fn scene_of(parts: &[(&str, &MeshPart)]) -> russimp_ng::scene::Scene {
    use russimp_ng::{
        face::Face,
        material::{Material, MaterialProperty, PropertyTypeInfo, TextureType},
        mesh::{Mesh, PrimitiveType},
        node::Node,
        Matrix4x4, Vector3D, AABB,
    };
    use std::{cell::RefCell, rc::Rc};
    let identity = Matrix4x4 {
        a1: 1.0, a2: 0.0, a3: 0.0, a4: 0.0,
        b1: 0.0, b2: 1.0, b3: 0.0, b4: 0.0,
        c1: 0.0, c2: 0.0, c3: 1.0, c4: 0.0,
        d1: 0.0, d2: 0.0, d3: 0.0, d4: 1.0,
    };
    let v3 = |[x, y, z]: [f32; 3]| Vector3D { x, y, z };
    let property = |key: &str, data: PropertyTypeInfo| MaterialProperty {
        key: key.into(),
        data,
        index: 0,
        semantic: TextureType::None,
    };
    let material = |name: &str| Material {
        properties: vec![
            property("?mat.name", PropertyTypeInfo::String(name.into())),
            property("$clr.diffuse", PropertyTypeInfo::FloatArray(vec![1.0, 1.0, 1.0, 1.0])),
        ],
        textures: Default::default(),
    };
    // Assimp's OBJ import puts a default material first; the parts' come
    // after it (their indices are what callers know them by).
    let mut materials = vec![material("DefaultMaterial")];
    let mut meshes = Vec::new();
    let mut children = Vec::new();
    for (name, part) in parts.iter().filter(|(_, part)| !part.is_empty()) {
        let index = meshes.len() as u32;
        materials.push(material(name));
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        for p in &part.positions {
            for i in 0..3 {
                min[i] = min[i].min(p[i]);
                max[i] = max[i].max(p[i]);
            }
        }
        meshes.push(Mesh {
            name: (*name).into(),
            vertices: part.positions.iter().copied().map(v3).collect(),
            normals: part.normals.iter().copied().map(v3).collect(),
            texture_coords: vec![Some(part.uvs.iter().map(|&[u, v]| Vector3D { x: u, y: 1.0 - v, z: 0.0 }).collect())],
            uv_components: vec![2],
            primitive_types: PrimitiveType::Triangle as u32,
            material_index: index + 1,
            faces: part.triangles.iter().map(|t| Face(t.to_vec())).collect(),
            aabb: AABB { min: v3(min), max: v3(max) },
            ..Default::default()
        });
        children.push(Rc::new(Node {
            name: (*name).into(),
            meshes: vec![index],
            transformation: identity,
            ..Default::default()
        }));
    }
    let root = Rc::new(Node {
        name: "generated".into(),
        children: RefCell::new(children),
        transformation: identity,
        ..Default::default()
    });
    russimp_ng::scene::Scene {
        materials,
        meshes,
        metadata: None,
        animations: Vec::new(),
        cameras: Vec::new(),
        lights: Vec::new(),
        root: Some(root),
        flags: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_obj_holds_every_part_and_triangle() {
        let mut a = MeshPart::default();
        let v = |part: &mut MeshPart, x: f32| part.vertex(Vector3::new(x, 0.0, 0.0), Vector3::unit_y(), [0.0, 0.0]);
        let (p, q, r, s) = (v(&mut a, 0.0), v(&mut a, 1.0), v(&mut a, 2.0), v(&mut a, 3.0));
        a.quad([p, q, r, s]);
        let empty = MeshPart::default();
        let path = write_obj(&[("stone", &a), ("nothing", &empty), ("road", &a)]).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().filter(|l| l.starts_with("f ")).count(), 4);
        assert!(text.contains("usemtl stone") && text.contains("usemtl road") && !text.contains("nothing"));
        // The second part's indices start after the first's vertices.
        assert!(text.contains("f 5/5/5"));
        let mtl = std::fs::read_to_string(path.with_extension("mtl")).unwrap();
        assert!(mtl.contains("newmtl road") && !mtl.contains("nothing"));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("mtl"));
    }

    /// The model made in memory matches the one assimp made of the OBJ.
    #[test]
    fn a_model_in_memory_matches_the_obj_one() {
        let mut walls = MeshPart::default();
        let mut roof = MeshPart::default();
        let v = |part: &mut MeshPart, x: f32, y: f32, u: f32| {
            part.vertex(Vector3::new(x, y, 0.5), Vector3::unit_z(), [u, 0.25])
        };
        let q: Vec<u32> = [(0.0, 0.0, 0.0), (1.0, 0.0, 1.0), (1.0, 1.0, 1.0), (0.0, 1.0, 0.0)]
            .iter()
            .map(|&(x, y, u)| v(&mut walls, x, y, u))
            .collect();
        walls.quad([q[0], q[1], q[2], q[3]]);
        let r: Vec<u32> = [(0.0, 2.0, 0.5), (1.0, 2.0, 0.5), (0.5, 3.0, 0.5)].iter().map(|&(x, y, u)| v(&mut roof, x, y, u)).collect();
        roof.triangles.push([r[0], r[1], r[2]]);
        let parts = [("walls", &walls), ("empty", &MeshPart::default()), ("roof", &roof)];

        let path = write_obj(&parts).unwrap();
        let mut from_obj = ModelBuilder::from_file(&path.to_string_lossy()).unwrap().with_scale(1.0).build();
        from_obj.init();
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("mtl"));
        let in_memory = build_model(&parts).unwrap();

        let names = |m: &Model| m.meshes.iter().map(|(n, mesh)| (n.clone(), mesh.indices.len(), mesh.material_index)).collect::<Vec<_>>();
        assert_eq!(names(&in_memory), names(&from_obj));
        assert_eq!(in_memory.material_count(), from_obj.material_count());
        for i in 0..in_memory.material_count() {
            assert_eq!(in_memory.material_name_at(i), from_obj.material_name_at(i));
        }
        // The OBJ import repeats vertices per corner: compare corners.
        for ((_, a), (_, b)) in in_memory.meshes.iter().zip(&from_obj.meshes) {
            let corners = |m: &crate::core::model::ModelMesh| {
                m.indices.iter().map(|&i| {
                    let v = &m.vertices[i as usize];
                    (v.position, v.normal, v.texture_coords)
                }).collect::<Vec<_>>()
            };
            for (x, y) in corners(a).iter().zip(&corners(b)) {
                let close = |p: (f32, f32, f32), q: (f32, f32, f32)| {
                    (p.0 - q.0).abs() < 1e-3 && (p.1 - q.1).abs() < 1e-3 && (p.2 - q.2).abs() < 1e-3
                };
                assert!(close(x.0, y.0) && close(x.1, y.1), "{x:?} {y:?}");
                assert!((x.2 .0 - y.2 .0).abs() < 1e-3 && (x.2 .1 - y.2 .1).abs() < 1e-3, "{x:?} {y:?}");
            }
        }
        let part = |m: &Model| m.node_part("roof").map(|(names, at)| (names, (at.x, at.y, at.z)));
        assert_eq!(part(&in_memory), part(&from_obj));
    }
}
