//! Meshes made in code (trees, bridges) turned into models: written to a
//! temporary OBJ file with one material per part and loaded like any model,
//! so they are drawn, shadowed and picked like one.

use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use cgmath::Vector3;

use super::{Model, ModelBuilder};

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

/// `parts` as a model (see `write_obj`), its materials named as the parts.
/// Needs a GPU.
pub fn build_model(parts: &[(&str, &MeshPart)]) -> Result<Model, String> {
    let path = write_obj(parts)?;
    let built = ModelBuilder::from_file(&path.to_string_lossy())
        .map_err(|e| format!("could not load a generated mesh: {e}"))
        .map(|builder| builder.with_scale(1.0).build());
    // The file is read: not needed any more.
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("mtl"));
    let mut model = built?;
    model.init();
    Ok(model)
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
}
