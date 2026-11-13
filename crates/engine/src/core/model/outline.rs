//! What a model file contains, read without any GPU work: the separately
//! usable objects ("variants", e.g. the trees of a tree pack) with their
//! LODs, and the materials. Used by the asset importer to describe a file
//! and by the loader to split it up the same way.

use std::{path::Path, rc::Rc};

use russimp_ng::{
    material::{Material, PropertyTypeInfo},
    metadata::MetadataType,
    node::Node,
    scene::Scene,
};

/// A model that would come out smaller than this (in metres, largest
/// dimension of its biggest variant) at its file's unit is taken to be
/// modelled in metres after all: exporters often label metre files as
/// centimetres.
const MIN_PLAUSIBLE_SIZE: f32 = 0.5;

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

pub struct ClipOutline {
    pub name: String,
    /// Length in seconds.
    pub duration: f32,
}

pub struct ModelOutline {
    /// Empty for a file that only holds animations.
    pub variants: Vec<VariantOutline>,
    /// In the file's material order.
    pub materials: Vec<MaterialOutline>,
    /// The animation clips in the file.
    pub clips: Vec<ClipOutline>,
    /// Metres per unit of the file, as the file states it (FBX). 1 when it
    /// does not say.
    pub unit: f32,
    /// Largest dimension of the biggest variant at its most detailed LOD,
    /// in file units. 0 without meshes.
    pub size: f32,
}

impl ModelOutline {
    /// Scale to import the model at so it comes out in metres: the file's
    /// unit, unless that makes the model implausibly small.
    pub fn import_scale(&self) -> f32 {
        if self.unit != 1.0 && self.size * self.unit >= MIN_PLAUSIBLE_SIZE {
            self.unit
        } else {
            1.0
        }
    }
}

/// Metres per file unit. FBX gives its unit as `UnitScaleFactor` in
/// centimetres (1 = cm, 100 = m); other formats are taken as metres.
fn file_unit(scene: &Scene) -> f32 {
    let Some(metadata) = &scene.metadata else {
        return 1.0;
    };
    let factor = metadata
        .keys
        .iter()
        .zip(&metadata.values)
        .find(|(key, _)| key.as_str() == "UnitScaleFactor")
        .and_then(|(_, value)| match &value.0 {
            Ok(MetadataType::Float(f)) => Some(*f as f64),
            Ok(MetadataType::Double(f)) => Some(*f),
            Ok(MetadataType::Int(i)) => Some(*i as f64),
            _ => None,
        });
    match factor {
        Some(factor) if factor > 0.0 => (factor / 100.0) as f32,
        _ => 1.0,
    }
}

/// Largest dimension of the meshes below `node` (node transforms are not
/// applied, as the loader does not apply them either).
fn node_size(scene: &Scene, node: &Node) -> f32 {
    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    extend_bounds(scene, node, &mut min, &mut max);
    (0..3)
        .map(|axis| max[axis] - min[axis])
        .filter(|d| d.is_finite() && *d >= 0.0)
        .fold(0.0, f32::max)
}

fn extend_bounds(scene: &Scene, node: &Node, min: &mut [f32; 3], max: &mut [f32; 3]) {
    for &index in &node.meshes {
        let Some(mesh) = scene.meshes.get(index as usize) else {
            continue;
        };
        for v in &mesh.vertices {
            for (axis, c) in [v.x, v.y, v.z].into_iter().enumerate() {
                min[axis] = min[axis].min(c);
                max[axis] = max[axis].max(c);
            }
        }
    }
    for child in node.children.borrow().iter() {
        extend_bounds(scene, child, min, max);
    }
}

/// Names of a file's animation clips, in file order and made unique:
/// exporters often leave them empty or give every clip the same name.
pub fn clip_names(scene: &Scene) -> Vec<String> {
    let mut names: Vec<String> = Vec::with_capacity(scene.animations.len());
    for (index, animation) in scene.animations.iter().enumerate() {
        let base = if animation.name.trim().is_empty() {
            format!("clip {}", index + 1)
        } else {
            animation.name.trim().to_string()
        };
        let mut name = base.clone();
        let mut copy = 2;
        while names.contains(&name) {
            name = format!("{base} {copy}");
            copy += 1;
        }
        names.push(name);
    }
    names
}

/// Sort key for an LOD's name. Lower numbers are higher detail.
pub fn lod_priority(name: &str) -> u32 {
    let lc = name.to_lowercase();
    if lc.contains("lod0") {
        0
    } else if lc.contains("lod1") {
        1
    } else if lc.contains("lod2") {
        2
    } else if lc.contains("lod3") || lc.contains("billboard") {
        3
    } else if lc.contains("high") {
        0
    } else if lc.contains("last") {
        2
    } else if lc.contains("low") {
        1
    } else {
        99
    }
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
    let groups = match &scene.root {
        Some(root) => group_nodes(root, &default_variant_name(path)),
        None => Vec::new(),
    };
    let size = groups
        .iter()
        .filter_map(|group| group.lods.iter().min_by_key(|(name, _)| lod_priority(name)))
        .map(|(_, node)| node_size(&scene, node))
        .fold(0.0, f32::max);
    let variants: Vec<VariantOutline> = groups
        .into_iter()
        .map(|group| VariantOutline {
            name: group.name,
            lods: group.lods.into_iter().map(|(name, _)| name).collect(),
        })
        .collect();
    let clips: Vec<ClipOutline> = clip_names(&scene)
        .into_iter()
        .zip(&scene.animations)
        .map(|(name, animation)| {
            // Assimp reports 0 ticks per second when the file does not say.
            let ticks = if animation.ticks_per_second > 0.0 {
                animation.ticks_per_second
            } else {
                25.0
            };
            ClipOutline {
                name,
                duration: (animation.duration / ticks) as f32,
            }
        })
        .collect();
    if variants.is_empty() && clips.is_empty() {
        return Err("the file contains neither meshes nor animations".to_string());
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
        clips,
        unit: file_unit(&scene),
        size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outline(unit: f32, size: f32) -> ModelOutline {
        ModelOutline {
            variants: Vec::new(),
            materials: Vec::new(),
            clips: Vec::new(),
            unit,
            size,
        }
    }

    #[test]
    fn a_centimetre_file_is_scaled_to_metres() {
        // A 113 cm boulder.
        assert_eq!(outline(0.01, 113.0).import_scale(), 0.01);
    }

    #[test]
    fn a_metre_model_labelled_as_centimetres_keeps_its_size() {
        // A 23.5 m tree that would shrink to 0.24 m.
        assert_eq!(outline(0.01, 23.5).import_scale(), 1.0);
    }

    #[test]
    fn a_file_without_a_unit_is_taken_as_metres() {
        assert_eq!(outline(1.0, 53.0).import_scale(), 1.0);
    }
}
