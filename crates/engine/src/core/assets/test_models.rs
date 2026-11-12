//! Tiny model and image files for tests.

use std::{fs, path::Path};

/// A five-vertex pyramid: 18 `u16` indices followed by 5 `f32` positions.
const PYRAMID: &str = "AAAEAAEAAQAEAAIAAgAEAAMAAwAEAAAAAAABAAIAAAACAAMAAADAvwAAAAAAAMC/AADAPwAAAAAAAMC/AADAPwAAAAAAAMA/AADAvwAAAAAAAMA/AAAAAAAAYEEAAAAA";

/// Write a glTF file whose scene has the given top-level nodes, each with
/// the given children. A node without children carries the pyramid mesh, a
/// node with children is a plain group and its children carry the mesh.
/// The one material is called `bark`; with `texture` it refers to that
/// image file as its colour map.
pub(crate) fn write_gltf(path: &Path, nodes: &[(&str, &[&str])], texture: Option<&str>) {
    let mut entries: Vec<String> = Vec::new();
    let mut scene_nodes: Vec<usize> = Vec::new();
    let mut next_child = nodes.len();
    for (index, (name, children)) in nodes.iter().enumerate() {
        scene_nodes.push(index);
        if children.is_empty() {
            entries.push(format!(r#"{{"name": "{name}", "mesh": 0}}"#));
        } else {
            let indices: Vec<String> = (next_child..next_child + children.len())
                .map(|i| i.to_string())
                .collect();
            next_child += children.len();
            entries.push(format!(
                r#"{{"name": "{name}", "children": [{}]}}"#,
                indices.join(", ")
            ));
        }
    }
    for (_, children) in nodes {
        for child in *children {
            entries.push(format!(r#"{{"name": "{child}", "mesh": 0}}"#));
        }
    }
    let scene_nodes: Vec<String> = scene_nodes.iter().map(|i| i.to_string()).collect();

    let (material, images) = match texture {
        Some(file) => (
            r#"{"name": "bark", "pbrMetallicRoughness": {"baseColorTexture": {"index": 0}}}"#
                .to_string(),
            format!(
                r#""textures": [{{"source": 0}}], "images": [{{"uri": "{}"}}],"#,
                file.replace(' ', "%20")
            ),
        ),
        None => (
            r#"{"name": "bark", "pbrMetallicRoughness": {"baseColorFactor": [0.2, 0.5, 0.2, 1.0]}}"#
                .to_string(),
            String::new(),
        ),
    };

    let text = format!(
        r#"{{
  "asset": {{"version": "2.0"}},
  "scene": 0,
  "scenes": [{{"nodes": [{scene_nodes}]}}],
  "nodes": [{nodes}],
  "materials": [{material}],
  {images}
  "meshes": [{{"primitives": [{{"attributes": {{"POSITION": 1}}, "indices": 0, "material": 0}}]}}],
  "buffers": [{{"byteLength": 96, "uri": "data:application/octet-stream;base64,{PYRAMID}"}}],
  "bufferViews": [
    {{"buffer": 0, "byteOffset": 0, "byteLength": 36, "target": 34963}},
    {{"buffer": 0, "byteOffset": 36, "byteLength": 60, "target": 34962}}
  ],
  "accessors": [
    {{"bufferView": 0, "componentType": 5123, "count": 18, "type": "SCALAR"}},
    {{"bufferView": 1, "componentType": 5126, "count": 5, "type": "VEC3",
      "min": [-1.5, 0.0, -1.5], "max": [1.5, 14.0, 1.5]}}
  ]
}}"#,
        scene_nodes = scene_nodes.join(", "),
        nodes = entries.join(", "),
    );
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder).unwrap();
    }
    fs::write(path, text).unwrap();
}

/// Write a 2x2 opaque PNG.
pub(crate) fn write_png(path: &Path) {
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder).unwrap();
    }
    image::RgbaImage::from_pixel(2, 2, image::Rgba([120, 90, 60, 255]))
        .save(path)
        .unwrap();
}

/// Two keyframe times (0 s, 1 s) followed by two translations.
const ANIMATION: &str = "AAAAAAAAgD8AAAAAAAAAAAAAAAAAAAAAAACAPwAAAAA=";

/// Write a glTF file without meshes that animates one node ("hips") with a
/// one-second clip of the given name.
pub(crate) fn write_animation_gltf(path: &Path, clip: &str) {
    let text = format!(
        r#"{{
  "asset": {{"version": "2.0"}},
  "scene": 0,
  "scenes": [{{"nodes": [0]}}],
  "nodes": [{{"name": "hips"}}],
  "animations": [{{
    "name": "{clip}",
    "samplers": [{{"input": 0, "output": 1, "interpolation": "LINEAR"}}],
    "channels": [{{"sampler": 0, "target": {{"node": 0, "path": "translation"}}}}]
  }}],
  "buffers": [{{"byteLength": 32, "uri": "data:application/octet-stream;base64,{ANIMATION}"}}],
  "bufferViews": [
    {{"buffer": 0, "byteOffset": 0, "byteLength": 8}},
    {{"buffer": 0, "byteOffset": 8, "byteLength": 24}}
  ],
  "accessors": [
    {{"bufferView": 0, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0.0], "max": [1.0]}},
    {{"bufferView": 1, "componentType": 5126, "count": 2, "type": "VEC3"}}
  ]
}}"#
    );
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder).unwrap();
    }
    fs::write(path, text).unwrap();
}
