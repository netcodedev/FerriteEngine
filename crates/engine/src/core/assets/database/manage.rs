//! Changing the set of assets on disk from the editor: who refers to an
//! asset, deleting, renaming and duplicating assets, and the folders that
//! hold them. See `docs/asset-browser.md`.

use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
};

use super::{AssetDatabase, AssetEntry, AssetError};
use crate::core::assets::{AssetId, AssetMeta, META_SUFFIX};

/// File name endings that belong to the engine's own formats and stay
/// together when a file is renamed (`Bark.material.ron`, not `Bark.ron`).
const COMPOUND_SUFFIXES: [&str; 4] = [".material.ron", ".matgraph.ron", ".scatter.ron", ".clip.ron"];

/// Files searched for references are at most this big; larger ones are
/// data, not settings.
const MAX_SEARCHED_FILE: u64 = 8 * 1024 * 1024;

/// Extensions of the text files that may name an asset.
const SEARCHED_EXTENSIONS: [&str; 4] = ["ron", "rs", "json", "toml"];

/// Folders of a project never searched for references.
const SKIPPED_FOLDERS: [&str; 3] = ["target", "saves", "node_modules"];

/// Where removed files go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Removal {
    /// The system's trash, where they can be restored from.
    Trash,
    /// Gone for good (tests).
    Permanent,
}

/// A file that refers to an asset.
#[derive(Clone, Debug, PartialEq)]
pub struct Reference {
    /// The file, as found below the project folder.
    pub file: PathBuf,
    /// The asset the file belongs to (as its source or sidecar), if any.
    pub asset: Option<AssetId>,
}

/// A file name split into the part to rename and the ending to keep.
pub fn split_file_name(name: &str) -> (&str, &str) {
    let lower = name.to_lowercase();
    for suffix in COMPOUND_SUFFIXES {
        if lower.ends_with(suffix) && name.len() > suffix.len() {
            return name.split_at(name.len() - suffix.len());
        }
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 => name.split_at(dot),
        _ => (name, ""),
    }
}

/// Whether `name` can be a file or folder name: not empty, no path parts.
fn valid_name(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|'])
        && !name.ends_with(META_SUFFIX)
}

fn io_error(path: &Path, e: io::Error) -> AssetError {
    AssetError::Io(path.to_path_buf(), e)
}

fn exists_error(path: &Path) -> AssetError {
    AssetError::Io(
        path.to_path_buf(),
        io::Error::new(io::ErrorKind::AlreadyExists, "a file or folder of that name exists already"),
    )
}

fn name_error(name: &str) -> AssetError {
    AssetError::Io(
        PathBuf::from(name),
        io::Error::new(io::ErrorKind::InvalidInput, "not a usable name"),
    )
}

/// Remove a file or folder the way `removal` says.
pub fn remove_path(path: &Path, removal: Removal) -> Result<(), AssetError> {
    match removal {
        Removal::Trash => trash::delete(path).map_err(|e| {
            AssetError::Io(path.to_path_buf(), io::Error::other(format!("could not move to the trash: {e}")))
        }),
        Removal::Permanent if path.is_dir() => fs::remove_dir_all(path).map_err(|e| io_error(path, e)),
        Removal::Permanent => fs::remove_file(path).map_err(|e| io_error(path, e)),
    }
}

impl AssetDatabase {
    /// The project folder: the one holding the asset root.
    pub fn project_root(&self) -> PathBuf {
        match self.root.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
            _ => PathBuf::from("."),
        }
    }

    /// Files of the project that name the asset: by id (sidecars, material
    /// files, scatter graphs, clips, scenes — anything that stores
    /// `"<id>"`) or by its path below the project (scripts). The asset's
    /// own source and sidecar do not count.
    pub fn references_to(&self, id: AssetId) -> Vec<Reference> {
        let Some(entry) = self.get(id) else {
            return Vec::new();
        };
        let own = [entry.source_path(), entry.meta_path()];
        let project = self.project_root();
        let path_text = entry
            .source_path()
            .strip_prefix(&project)
            .unwrap_or(&entry.source_path())
            .to_string_lossy()
            .replace('\\', "/");
        let needles = [format!("\"{id}\""), path_text];
        self.files_naming(&needles, |file| !own.iter().any(|o| same_file(o, file)))
    }

    /// Files outside `folder` that name an asset inside it.
    pub fn references_into(&self, folder: &Path) -> Vec<Reference> {
        let inside: Vec<&AssetEntry> = self.iter().filter(|e| e.dir.starts_with(folder)).collect();
        let needles: Vec<String> = inside.iter().map(|e| format!("\"{}\"", e.meta.id)).collect();
        if needles.is_empty() {
            return Vec::new();
        }
        self.files_naming(&needles, |file| !file.starts_with(folder))
    }

    /// Text files below the project folder containing any of `needles`,
    /// among those `keep` accepts, in path order.
    fn files_naming(&self, needles: &[String], keep: impl Fn(&Path) -> bool) -> Vec<Reference> {
        let owners = self.file_owners();
        let mut found = Vec::new();
        let mut pending = vec![self.project_root()];
        while let Some(dir) = pending.pop() {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
            paths.sort();
            for path in paths {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if path.is_dir() {
                    if !name.starts_with('.') && !SKIPPED_FOLDERS.contains(&name) {
                        pending.push(path);
                    }
                    continue;
                }
                let searched = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| SEARCHED_EXTENSIONS.contains(&e.to_lowercase().as_str()));
                let small = fs::metadata(&path).is_ok_and(|m| m.len() <= MAX_SEARCHED_FILE);
                if !searched || !small || !keep(&path) {
                    continue;
                }
                let Ok(text) = fs::read_to_string(&path) else {
                    continue;
                };
                if needles.iter().any(|needle| !needle.is_empty() && text.contains(needle.as_str())) {
                    let asset = owners.get(&normalize(&path)).copied();
                    found.push(Reference { file: path, asset });
                }
            }
        }
        found.sort_by(|a, b| a.file.cmp(&b.file));
        found
    }

    /// Which asset each source and sidecar file belongs to.
    fn file_owners(&self) -> HashMap<PathBuf, AssetId> {
        let mut owners = HashMap::new();
        for entry in self.iter() {
            owners.insert(normalize(&entry.source_path()), entry.meta.id);
            owners.insert(normalize(&entry.meta_path()), entry.meta.id);
        }
        owners
    }

    /// Remove an asset: its source file and sidecar go (to the trash or for
    /// good) and it leaves the database. Other files next to it (a glTF's
    /// buffers) stay. What refers to it is not changed; ask
    /// `references_to` first.
    pub fn delete(&mut self, id: AssetId, removal: Removal) -> Result<(), AssetError> {
        let entry = self.get(id).ok_or(AssetError::UnknownAsset(id))?.clone();
        let source = entry.source_path();
        if source.exists() {
            remove_path(&source, removal)?;
        }
        let meta = entry.meta_path();
        if meta.exists() {
            remove_path(&meta, removal)?;
        }
        self.assets.remove(&id);
        self.changes += 1;
        Ok(())
    }

    /// Give an asset a new name: its display name, and its source file and
    /// sidecar the same name (the file's ending kept). What refers to it by
    /// id stays intact.
    pub fn rename(&mut self, id: AssetId, name: &str) -> Result<(), AssetError> {
        let name = name.trim();
        if !valid_name(name) {
            return Err(name_error(name));
        }
        let entry = self.get(id).ok_or(AssetError::UnknownAsset(id))?.clone();
        let (_, ending) = split_file_name(&entry.meta.source);
        let file_name = format!("{name}{ending}");
        let mut meta = entry.meta.clone();
        meta.name = name.to_string();
        if file_name != entry.meta.source {
            let from = entry.source_path();
            let to = entry.dir.join(&file_name);
            // Only a change of case on a case-blind file system finds itself.
            if to.exists() && !same_file(&from, &to) {
                return Err(exists_error(&to));
            }
            if from.exists() {
                fs::rename(&from, &to).map_err(|e| io_error(&from, e))?;
            }
            meta.source = file_name;
        }
        // Writes the new sidecar and removes the old one.
        self.update(meta)
    }

    /// A copy of an asset next to it, named "<name> copy" (or "copy 2", ...),
    /// with a new id. Returns the copy's id.
    pub fn duplicate(&mut self, id: AssetId) -> Result<AssetId, AssetError> {
        let entry = self.get(id).ok_or(AssetError::UnknownAsset(id))?.clone();
        let (_, ending) = split_file_name(&entry.meta.source);
        let name = (1..)
            .map(|n| if n == 1 { format!("{} copy", entry.meta.name) } else { format!("{} copy {n}", entry.meta.name) })
            .find(|n| !entry.dir.join(format!("{n}{ending}")).exists())
            .expect("a free name");
        let source = format!("{name}{ending}");
        let from = entry.source_path();
        let to = entry.dir.join(&source);
        fs::copy(&from, &to).map_err(|e| io_error(&from, e))?;
        let meta = AssetMeta {
            id: AssetId::new(),
            name,
            source,
            ..entry.meta.clone()
        };
        self.register(meta, &entry.dir)
    }

    /// A new, empty folder `name` inside `parent` (which must be inside the
    /// asset root). Returns its path.
    pub fn create_folder(&mut self, parent: &Path, name: &str) -> Result<PathBuf, AssetError> {
        let name = name.trim();
        if !valid_name(name) {
            return Err(name_error(name));
        }
        let path = parent.join(name);
        if path.exists() {
            return Err(exists_error(&path));
        }
        fs::create_dir_all(&path).map_err(|e| io_error(&path, e))?;
        self.changes += 1;
        Ok(path)
    }

    /// Rename a folder below the asset root; the assets in it move along.
    /// Returns the new path.
    pub fn rename_folder(&mut self, folder: &Path, name: &str) -> Result<PathBuf, AssetError> {
        let name = name.trim();
        if !valid_name(name) {
            return Err(name_error(name));
        }
        if !self.is_below_root(folder) {
            return Err(name_error(&folder.to_string_lossy()));
        }
        let to = folder.with_file_name(name);
        if to.exists() && !same_file(folder, &to) {
            return Err(exists_error(&to));
        }
        fs::rename(folder, &to).map_err(|e| io_error(folder, e))?;
        self.move_entries(folder, &to);
        Ok(to)
    }

    /// Remove a folder below the asset root with everything in it.
    pub fn delete_folder(&mut self, folder: &Path, removal: Removal) -> Result<(), AssetError> {
        if !self.is_below_root(folder) {
            return Err(name_error(&folder.to_string_lossy()));
        }
        remove_path(folder, removal)?;
        self.assets.retain(|_, entry| !entry.dir.starts_with(folder));
        self.changes += 1;
        Ok(())
    }

    /// The assets in a folder and the folders below it.
    pub fn assets_in(&self, folder: &Path) -> Vec<AssetId> {
        self.iter().filter(|e| e.dir.starts_with(folder)).map(|e| e.meta.id).collect()
    }

    /// Strictly inside the asset root (not the root itself).
    fn is_below_root(&self, path: &Path) -> bool {
        path.starts_with(&self.root) && path != self.root
    }

    /// Entries whose folder was `from` (or below) now live below `to`.
    fn move_entries(&mut self, from: &Path, to: &Path) {
        for entry in self.assets.values_mut() {
            if let Ok(rest) = entry.dir.strip_prefix(from) {
                entry.dir = to.join(rest);
                // A path is part of what makes up a loaded asset.
                entry.revision = self.next_revision;
                self.next_revision += 1;
            }
        }
        self.changes += 1;
    }
}

/// Paths compared without `./` and the like, as far as they exist.
fn normalize(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.components().collect())
}

fn same_file(a: &Path, b: &Path) -> bool {
    normalize(a) == normalize(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::assets::{
        database::tests::TempDir, AssetKind, MaterialAssetSettings, MaterialGraphSettings, MaterialSettings,
        ModelSettings, ScatterGraphSettings, TextureSettings, ASSET_META_VERSION,
    };

    fn meta(name: &str, source: &str, kind: AssetKind) -> AssetMeta {
        AssetMeta {
            version: ASSET_META_VERSION,
            id: AssetId::new(),
            name: name.into(),
            source: source.into(),
            kind,
        }
    }

    /// A project with an asset root and a few assets referring to each other:
    /// a model slot uses a material, the material uses a graph and a
    /// texture, a scatter graph plants the model, a scene places it.
    struct Project {
        _temp: TempDir,
        project: PathBuf,
        database: AssetDatabase,
        graph: AssetId,
        texture: AssetId,
        material: AssetId,
        model: AssetId,
        scatter: AssetId,
    }

    fn project() -> Project {
        let temp = TempDir::new("assets-manage");
        let project = temp.0.clone();
        let root = project.join("assets");
        let materials = root.join("materials");
        let models = root.join("models").join("rock");
        let textures = root.join("textures");
        let scatter = root.join("scatter");
        for dir in [&materials, &models, &textures, &scatter, &project.join("scenes")] {
            fs::create_dir_all(dir).unwrap();
        }
        let mut database = AssetDatabase::new(&root);

        fs::write(materials.join("Rough.matgraph.ron"), "(nodes: [])").unwrap();
        let graph = database
            .register(meta("Rough", "Rough.matgraph.ron", AssetKind::MaterialGraph(MaterialGraphSettings {})), &materials)
            .unwrap();
        fs::write(textures.join("stone.png"), b"png").unwrap();
        let texture = database
            .register(meta("stone", "stone.png", AssetKind::Texture(TextureSettings {})), &textures)
            .unwrap();
        fs::write(
            materials.join("Stone.material.ron"),
            format!("(graph: Asset(\"{graph}\"), params: {{\"base_color\": Texture(Asset(\"{texture}\"))}})"),
        )
        .unwrap();
        let material = database
            .register(meta("Stone", "Stone.material.ron", AssetKind::Material(MaterialAssetSettings {})), &materials)
            .unwrap();
        fs::write(models.join("rock.glb"), b"glb").unwrap();
        let model = database
            .register(
                meta(
                    "rock",
                    "rock.glb",
                    AssetKind::Model(ModelSettings {
                        materials: vec![MaterialSettings {
                            name: "Stone".into(),
                            material: Some(material),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                ),
                &models,
            )
            .unwrap();
        fs::write(scatter.join("Rocks.scatter.ron"), format!("(asset: Some((id: \"{model}\")))")).unwrap();
        let scatter = database
            .register(meta("Rocks", "Rocks.scatter.ron", AssetKind::ScatterGraph(ScatterGraphSettings {})), &scatter)
            .unwrap();
        fs::write(project.join("scenes").join("main.ron"), format!("(model: \"{model}\")")).unwrap();
        Project { _temp: temp, project, database, graph, texture, material, model, scatter }
    }

    fn referrers(p: &Project, id: AssetId) -> Vec<Option<AssetId>> {
        p.database.references_to(id).into_iter().map(|r| r.asset).collect()
    }

    #[test]
    fn references_are_found_across_kinds_but_not_in_the_assets_own_files() {
        let p = project();
        // Material → graph and texture.
        assert_eq!(referrers(&p, p.graph), vec![Some(p.material)]);
        assert_eq!(referrers(&p, p.texture), vec![Some(p.material)]);
        // Model slot → material (in the model's sidecar).
        assert_eq!(referrers(&p, p.material), vec![Some(p.model)]);
        // Scatter graph and a scene → model.
        let model_refs = p.database.references_to(p.model);
        assert_eq!(model_refs.len(), 2, "{model_refs:?}");
        assert_eq!(model_refs[0].asset, Some(p.scatter));
        assert_eq!(model_refs[1].asset, None);
        assert!(model_refs[1].file.ends_with("scenes/main.ron"));
        // Nothing points at the scatter graph.
        assert!(p.database.references_to(p.scatter).is_empty());
    }

    #[test]
    fn a_script_naming_the_path_counts_as_a_reference() {
        let p = project();
        fs::create_dir_all(p.project.join("scripts")).unwrap();
        fs::write(p.project.join("scripts").join("rocks.rs"), "load(\"assets/textures/stone.png\")").unwrap();
        let refs = p.database.references_to(p.texture);
        assert_eq!(refs.len(), 2, "{refs:?}");
        assert!(refs.iter().any(|r| r.file.ends_with("scripts/rocks.rs") && r.asset.is_none()));
    }

    #[test]
    fn deleting_removes_source_sidecar_and_entry() {
        let mut p = project();
        let entry = p.database.get(p.graph).unwrap().clone();
        let before = p.database.changes();
        p.database.delete(p.graph, Removal::Permanent).unwrap();
        assert!(!entry.source_path().exists());
        assert!(!entry.meta_path().exists());
        assert!(p.database.get(p.graph).is_none());
        assert!(p.database.changes() > before);
        // A fresh scan agrees, and the rest is untouched.
        let mut fresh = AssetDatabase::new(p.database.root());
        assert!(fresh.scan().is_empty());
        assert!(fresh.get(p.graph).is_none());
        assert_eq!(fresh.len(), 4);
        // Deleting it again fails cleanly.
        assert!(matches!(p.database.delete(p.graph, Removal::Permanent), Err(AssetError::UnknownAsset(_))));
    }

    #[test]
    fn deleting_leaves_a_missing_source_and_other_files_alone() {
        let mut p = project();
        let entry = p.database.get(p.model).unwrap().clone();
        fs::write(entry.dir.join("rock.bin"), b"buffers").unwrap();
        fs::remove_file(entry.source_path()).unwrap();
        p.database.delete(p.model, Removal::Permanent).unwrap();
        assert!(!entry.meta_path().exists());
        assert!(entry.dir.join("rock.bin").exists());
    }

    #[test]
    fn renaming_moves_file_and_sidecar_and_keeps_references() {
        let mut p = project();
        p.database.rename(p.material, "Wet stone").unwrap();
        let entry = p.database.get(p.material).unwrap().clone();
        assert_eq!(entry.meta.name, "Wet stone");
        assert_eq!(entry.meta.source, "Wet stone.material.ron");
        assert!(entry.source_path().is_file());
        assert!(entry.meta_path().is_file());
        assert!(!entry.dir.join("Stone.material.ron").exists());
        assert!(!entry.dir.join(format!("Stone.material.ron{META_SUFFIX}")).exists());
        // The model still names it by id.
        assert_eq!(referrers(&p, p.material), vec![Some(p.model)]);
        let mut fresh = AssetDatabase::new(p.database.root());
        assert!(fresh.scan().is_empty());
        assert_eq!(fresh.get(p.material).unwrap().meta.name, "Wet stone");
    }

    #[test]
    fn renaming_onto_an_existing_file_or_with_a_bad_name_fails() {
        let mut p = project();
        let dir = p.database.get(p.material).unwrap().dir.clone();
        fs::write(dir.join("Taken.material.ron"), "()").unwrap();
        assert!(p.database.rename(p.material, "Taken").is_err());
        assert!(p.database.rename(p.material, "a/b").is_err());
        assert!(p.database.rename(p.material, "  ").is_err());
        assert_eq!(p.database.get(p.material).unwrap().meta.name, "Stone");
        assert!(dir.join("Stone.material.ron").is_file());
    }

    #[test]
    fn duplicating_makes_a_new_asset_next_to_the_old_one() {
        let mut p = project();
        let copy = p.database.duplicate(p.material).unwrap();
        let entry = p.database.get(copy).unwrap();
        assert_ne!(copy, p.material);
        assert_eq!(entry.meta.name, "Stone copy");
        assert_eq!(entry.meta.source, "Stone copy.material.ron");
        assert!(entry.source_path().is_file());
        let again = p.database.duplicate(p.material).unwrap();
        assert_eq!(p.database.get(again).unwrap().meta.name, "Stone copy 2");
    }

    #[test]
    fn folders_are_made_renamed_and_deleted_with_their_assets() {
        let mut p = project();
        let root = p.database.root().to_path_buf();
        let made = p.database.create_folder(&root, "props").unwrap();
        assert!(made.is_dir());
        assert!(p.database.create_folder(&root, "props").is_err());

        let models = root.join("models");
        let renamed = p.database.rename_folder(&models, "meshes").unwrap();
        let entry = p.database.get(p.model).unwrap();
        assert_eq!(entry.dir, renamed.join("rock"));
        assert!(entry.source_path().is_file());

        // The scatter graph and scene outside refer into the folder.
        let refs = p.database.references_into(&renamed);
        assert_eq!(refs.len(), 2, "{refs:?}");
        assert_eq!(p.database.assets_in(&renamed), vec![p.model]);

        p.database.delete_folder(&renamed, Removal::Permanent).unwrap();
        assert!(!renamed.exists());
        assert!(p.database.get(p.model).is_none());
        // The root itself cannot go.
        assert!(p.database.delete_folder(&root, Removal::Permanent).is_err());
    }

    #[test]
    fn file_names_keep_their_compound_ending() {
        assert_eq!(split_file_name("Bark.material.ron"), ("Bark", ".material.ron"));
        assert_eq!(split_file_name("A.B.matgraph.ron"), ("A.B", ".matgraph.ron"));
        assert_eq!(split_file_name("tree.gltf"), ("tree", ".gltf"));
        assert_eq!(split_file_name("README"), ("README", ""));
    }
}
