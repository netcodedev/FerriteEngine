//! New, empty assets of the kinds the engine writes itself, made from the
//! editor's "New" menu (see `docs/asset-browser.md`).

use std::path::Path;

use super::{AssetDatabase, AssetId};
use crate::{
    core::material::{
        asset::{MaterialFile, MATERIAL_SUFFIX},
        graph::MATERIAL_GRAPH_SUFFIX,
        standard, AlphaMode,
    },
    terrain::scatter::graph::{NodeKind, ScatterGraph, SCATTER_GRAPH_SUFFIX},
};

/// A kind of asset that can be made from nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NewAsset {
    /// A material on Standard PBR with its defaults.
    Material,
    /// A material graph: a copy of Standard PBR to change.
    MaterialGraph,
    /// A scatter graph planting nothing yet on random spots.
    ScatterGraph,
}

impl NewAsset {
    pub const ALL: [NewAsset; 3] = [NewAsset::Material, NewAsset::MaterialGraph, NewAsset::ScatterGraph];

    pub fn label(self) -> &'static str {
        match self {
            NewAsset::Material => "Material",
            NewAsset::MaterialGraph => "Material graph",
            NewAsset::ScatterGraph => "Scatter graph",
        }
    }

    /// Name for tools: `material`, `material_graph`, `scatter_graph`.
    pub fn key(self) -> &'static str {
        match self {
            NewAsset::Material => "material",
            NewAsset::MaterialGraph => "material_graph",
            NewAsset::ScatterGraph => "scatter_graph",
        }
    }

    pub fn from_key(key: &str) -> Option<NewAsset> {
        NewAsset::ALL.into_iter().find(|k| k.key() == key)
    }

    pub fn suffix(self) -> &'static str {
        match self {
            NewAsset::Material => MATERIAL_SUFFIX,
            NewAsset::MaterialGraph => MATERIAL_GRAPH_SUFFIX,
            NewAsset::ScatterGraph => SCATTER_GRAPH_SUFFIX,
        }
    }

    /// Folder below the asset root used when none is given.
    pub fn default_folder(self) -> &'static str {
        match self {
            NewAsset::Material | NewAsset::MaterialGraph => "materials",
            NewAsset::ScatterGraph => "scatter",
        }
    }

    /// First words of the names given when none is.
    fn default_name(self) -> &'static str {
        match self {
            NewAsset::Material => "New material",
            NewAsset::MaterialGraph => "New material graph",
            NewAsset::ScatterGraph => "New scatter graph",
        }
    }
}

/// Make a new asset of `kind` in `folder` (default: the kind's folder below
/// the asset root), named `name` or the first free "New material N".
/// Returns its id.
pub fn create(
    database: &mut AssetDatabase,
    kind: NewAsset,
    folder: Option<&Path>,
    name: Option<&str>,
) -> Result<AssetId, String> {
    let folder = match folder {
        Some(folder) => folder.to_path_buf(),
        None => database.root().join(kind.default_folder()),
    };
    if !folder.starts_with(database.root()) {
        return Err(format!("{} is not inside the asset folder", folder.display()));
    }
    std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let suffix = kind.suffix();
    let stem = match name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(name) => {
            if name.contains(['/', '\\']) {
                return Err(format!("'{name}' is not a usable name"));
            }
            if folder.join(format!("{name}{suffix}")).exists() {
                return Err(format!("'{name}' exists already"));
            }
            name.to_string()
        }
        None => (1..)
            .map(|n| format!("{} {n}", kind.default_name()))
            .find(|n| !folder.join(format!("{n}{suffix}")).exists())
            .expect("a free name"),
    };
    let path = folder.join(format!("{stem}{suffix}"));
    match kind {
        NewAsset::Material => MaterialFile { alpha: AlphaMode::Opaque, ..Default::default() }.save(&path)?,
        NewAsset::MaterialGraph => standard::standard_pbr().save(&path)?,
        NewAsset::ScatterGraph => starter_scatter_graph().save(&path)?,
    }
    database.import(&path).map_err(|e| e.to_string())
}

/// Random spots into a mesh spawner without a model: the smallest graph
/// that does something once a model is chosen.
fn starter_scatter_graph() -> ScatterGraph {
    let mut graph = ScatterGraph::random_layer(None, 100.0, "", rand::random());
    // Keep the random spots and the spawner; the world filters come later.
    graph.nodes.retain(|n| !matches!(n.kind, NodeKind::WorldChance { .. } | NodeKind::DensityCut { .. }));
    let ids: Vec<_> = graph.nodes.iter().map(|n| n.id).collect();
    graph.wires.clear();
    graph.connect(ids[0], 0, ids[1], 0);
    graph.nodes[1].position = [260.0, 0.0];
    graph
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::assets::{database::tests::TempDir, AssetKind};

    #[test]
    fn each_kind_is_made_in_the_given_folder_with_a_free_name() {
        let temp = TempDir::new("assets-create");
        let root = temp.0.join("assets");
        std::fs::create_dir_all(&root).unwrap();
        let mut database = AssetDatabase::new(&root);
        let props = root.join("props");
        for kind in NewAsset::ALL {
            let first = create(&mut database, kind, Some(&props), None).unwrap();
            let second = create(&mut database, kind, Some(&props), None).unwrap();
            let entry = database.get(second).unwrap();
            assert_eq!(entry.dir, props);
            assert_eq!(entry.meta.name, format!("{} 2", kind.default_name()));
            assert!(database.get(first).is_some());
            let matches = match (kind, &entry.meta.kind) {
                (NewAsset::Material, AssetKind::Material(_)) => true,
                (NewAsset::MaterialGraph, AssetKind::MaterialGraph(_)) => true,
                (NewAsset::ScatterGraph, AssetKind::ScatterGraph(_)) => true,
                _ => false,
            };
            assert!(matches, "{kind:?} made {:?}", entry.meta.kind);
        }
        // Default folder, a chosen name, and that name again.
        let rock = create(&mut database, NewAsset::Material, None, Some("Rock")).unwrap();
        assert_eq!(database.get(rock).unwrap().dir, root.join("materials"));
        assert!(create(&mut database, NewAsset::Material, None, Some("Rock")).is_err());
        assert!(create(&mut database, NewAsset::Material, Some(&temp.0), None).is_err());
    }
}
