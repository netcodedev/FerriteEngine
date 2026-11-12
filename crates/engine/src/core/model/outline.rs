//! What a model file contains, read without any GPU work: the separately
//! usable objects ("variants", e.g. the trees of a tree pack) with their
//! LODs, and the materials. Used by the asset importer to describe a file
//! and by the loader to split it up the same way.

use std::{path::Path, rc::Rc};

use russimp_ng::{
    material::{Material, PropertyTypeInfo},
    node::Node,
    scene::Scene,
};

/// One variant: a named group of scene-graph nodes, one per LOD.
pub struct NodeGroup {
    pub name: String,
    /// `(LOD name, node)` in scene-graph order; each node's subtree holds
    /// the meshes of that LOD.
    pub lods: Vec<(String, Rc<Node>)>,
}

/// Split a scene graph into variants by the names of its nodes:
///
/// * The root's children are LODs (`..._LOD0`, `high`, `low`, `billboard`,
///   ...): one variant, named `default_name`, with those LODs.
/// * Otherwise each child of the root is a variant, and the LOD-named
///   nodes below it are its LODs.
/// * No LOD names at all: one variant, named `default_name`, with a single
///   LOD holding everything.
///
/// Nodes without meshes are left out. Variant names are unique.
pub fn group_nodes(root: &Rc<Node>, default_name: &str) -> Vec<NodeGroup> {
    let children = root.children.borrow();
    let lod_named = children
        .iter()
        .filter(|c| is_lod_node_name(&c.name))
        .count();
    let single_variant = lod_named > 0 && lod_named * 2 >= children.len();

    let mut groups = Vec::new();
    if single_variant {
        let lods: Vec<(String, Rc<Node>)> = children
            .iter()
            .filter(|c| has_meshes(c))
            .map(|c| (c.name.clone(), c.clone()))
            .collect();
        if !lods.is_empty() {
            groups.push(NodeGroup {
                name: default_name.to_string(),
                lods,
            });
        }
    } else {
        for child in children.iter() {
            let mut lods = Vec::new();
            collect_lods_within(child, &mut lods);
            if !lods.is_empty() {
                groups.push(NodeGroup {
                    name: child.name.clone(),
                    lods,
                });
            }
        }
    }
    if groups.is_empty() && has_meshes(root) {
        groups.push(NodeGroup {
            name: default_name.to_string(),
            lods: vec![("lod0".to_string(), root.clone())],
        });
    }
    make_names_unique(&mut groups);
    groups
}

fn collect_lods_within(node: &Rc<Node>, out: &mut Vec<(String, Rc<Node>)>) {
    for child in node.children.borrow().iter() {
        if is_lod_node_name(&child.name) {
            if has_meshes(child) {
                out.push((child.name.clone(), child.clone()));
            }
        } else {
            // Some packs wrap LOD nodes inside extra grouping nodes before
            // reaching the meshes.
            collect_lods_within(child, out);
        }
    }
}

fn has_meshes(node: &Node) -> bool {
    !node.meshes.is_empty() || node.children.borrow().iter().any(|c| has_meshes(c))
}

fn make_names_unique(groups: &mut [NodeGroup]) {
    for index in 0..groups.len() {
        if groups[index].name.trim().is_empty() {
            groups[index].name = format!("variant {}", index + 1);
        }
        let mut name = groups[index].name.clone();
        let mut copy = 2;
        while groups[..index].iter().any(|g| g.name == name) {
            name = format!("{} {copy}", groups[index].name);
            copy += 1;
        }
        groups[index].name = name;
    }
}

pub fn is_lod_node_name(name: &str) -> bool {
    let lc = name.to_lowercase();
    lc.contains("lod")
        || lc.contains("high")
        || lc.ends_with("low")
        || lc.contains(" low")
        || lc.contains("_low")
        || lc.contains("last")
        || lc.contains("billboard")
}

/// The material's display name (`?mat.name`, assimp's standard key).
/// `None` for unnamed materials.
pub fn material_name(material: &Material) -> Option<String> {
    material.properties.iter().find_map(|p| {
        if p.key != "?mat.name" {
            return None;
        }
        if let PropertyTypeInfo::String(s) = &p.data {
            Some(s.clone())
        } else {
            None
        }
    })
}

/// Whether a material name reads as leaves rather than something solid.
pub fn looks_like_foliage(material_name: &str) -> bool {
    let lc = material_name.to_lowercase();
    lc.contains("cluster")
        || lc.contains("leaf")
        || lc.contains("leaves")
        || lc.contains("leav") // Paine pine names its foliage material "leav"
        || lc.contains("foliage")
        || lc.contains("billboard")
}

/// File name of a texture reference. Extracted by hand rather than via
/// `Path::file_name`, so Windows-style references like
/// `F:\artist\bark.png` also resolve on Linux.
pub fn texture_file_name(reference: &str) -> Option<&str> {
    let trimmed = reference.trim_end_matches('\0').trim();
    let name = trimmed.rsplit(['/', '\\']).next()?;
    (!name.is_empty()).then_some(name)
}

pub struct VariantOutline {
    pub name: String,
    pub lods: Vec<String>,
}

pub struct MaterialOutline {
    /// Empty for an unnamed material.
    pub name: String,
    /// File names of the textures the material refers to (not embedded
    /// ones), without folders.
    pub texture_files: Vec<String>,
}

pub struct ModelOutline {
    pub variants: Vec<VariantOutline>,
    /// In the file's material order.
    pub materials: Vec<MaterialOutline>,
}

/// File stem used as the name of a model's only variant.
pub fn default_variant_name(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "model".to_string())
}

/// Read a model file's outline. Needs no GL context.
pub fn inspect(path: &Path) -> Result<ModelOutline, String> {
    let scene = Scene::from_file(&path.to_string_lossy(), vec![]).map_err(|e| e.to_string())?;
    let variants = match &scene.root {
        Some(root) => group_nodes(root, &default_variant_name(path))
            .into_iter()
            .map(|group| VariantOutline {
                name: group.name,
                lods: group.lods.into_iter().map(|(name, _)| name).collect(),
            })
            .collect(),
        None => Vec::new(),
    };
    if variants.is_empty() {
        return Err("the file contains no meshes".to_string());
    }
    let materials = scene
        .materials
        .iter()
        .map(|material| {
            let mut texture_files: Vec<String> = Vec::new();
            for property in &material.properties {
                if property.key != "$tex.file" {
                    continue;
                }
                let PropertyTypeInfo::String(reference) = &property.data else {
                    continue;
                };
                // `*0` and the like point at textures embedded in the file.
                if reference.starts_with('*') {
                    continue;
                }
                if let Some(name) = texture_file_name(reference) {
                    if !texture_files.iter().any(|f| f == name) {
                        texture_files.push(name.to_string());
                    }
                }
            }
            MaterialOutline {
                name: material_name(material).unwrap_or_default(),
                texture_files,
            }
        })
        .collect();
    Ok(ModelOutline {
        variants,
        materials,
    })
}
