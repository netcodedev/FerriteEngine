//! Turning a file into an asset: copy it into the project (unless it is
//! already there), find out what it contains and write its sidecar.

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::core::model::outline;

use super::{
    AssetDatabase, AssetError, AssetId, AssetKind, AssetMeta, MaterialSettings, ModelSettings,
    TextureSettings, VariantInfo, ASSET_META_VERSION,
};

const MODEL_EXTENSIONS: [&str; 5] = ["gltf", "glb", "fbx", "obj", "dae"];
const TEXTURE_EXTENSIONS: [&str; 7] = ["png", "jpg", "jpeg", "tga", "bmp", "tif", "tiff"];

/// Folders below the asset root that imported files are copied into.
const MODEL_FOLDER: &str = "models";
const TEXTURE_FOLDER: &str = "textures";

#[derive(Clone, Copy, PartialEq, Eq)]
enum FileKind {
    Model,
    Texture,
}

fn file_kind(path: &Path) -> Option<FileKind> {
    let extension = path.extension()?.to_str()?.to_lowercase();
    if MODEL_EXTENSIONS.contains(&extension.as_str()) {
        Some(FileKind::Model)
    } else if TEXTURE_EXTENSIONS.contains(&extension.as_str()) {
        Some(FileKind::Texture)
    } else {
        None
    }
}

impl AssetDatabase {
    /// File extensions `import` accepts, lower case, for file dialogs.
    pub fn importable_extensions() -> Vec<&'static str> {
        MODEL_EXTENSIONS
            .iter()
            .chain(TEXTURE_EXTENSIONS.iter())
            .copied()
            .collect()
    }

    /// Register `file` as an asset and return its id.
    ///
    /// A file outside the asset root is copied into it first — a model into
    /// a folder of its own, together with the files it refers to (texture
    /// images, glTF buffers, OBJ material libraries). A file inside the
    /// root is registered where it is. Texture files a model refers to
    /// become texture assets too. Importing a file that already is an asset
    /// returns that asset.
    pub fn import(&mut self, file: &Path) -> Result<AssetId, AssetError> {
        let kind = file_kind(file).ok_or_else(|| AssetError::Unsupported(file.to_path_buf()))?;
        if !file.is_file() {
            return Err(AssetError::Io(
                file.to_path_buf(),
                std::io::Error::new(std::io::ErrorKind::NotFound, "no such file"),
            ));
        }
        if let Some(existing) = self.find_by_source(file) {
            return Ok(existing);
        }
        match kind {
            FileKind::Texture => {
                let source = match self.path_in_root(file) {
                    Some(inside) => inside,
                    None => {
                        let folder = self.root().join(TEXTURE_FOLDER);
                        copy_into(file, &folder, true)?
                    }
                };
                self.register_texture(&source)
            }
            FileKind::Model => self.import_model(file),
        }
    }

    fn import_model(&mut self, file: &Path) -> Result<AssetId, AssetError> {
        let name = outline::default_variant_name(file);
        let source_dir = file.parent().unwrap_or(Path::new(".")).to_path_buf();

        // Describe the file before anything is copied, so a file that
        // cannot be read leaves nothing behind.
        let found = outline::inspect(file).map_err(|e| AssetError::Import(file.to_path_buf(), e))?;

        let (source, copied) = match self.path_in_root(file) {
            Some(inside) => (inside, false),
            None => {
                let folder = unused_folder(&self.root().join(MODEL_FOLDER), &name);
                (copy_into(file, &folder, false)?, true)
            }
        };
        let target_dir = source.parent().unwrap_or(Path::new(".")).to_path_buf();

        if copied {
            // Files the model cannot be loaded without.
            for companion in companion_files(file) {
                let from = source_dir.join(&companion);
                if from.is_file() {
                    copy_to(&from, &target_dir.join(&companion))?;
                }
            }
        }

        // Textures: the ones the materials refer to, plus the ones the
        // loader would pick up by material name. They are copied next to
        // the model (where the loader looks first) and registered.
        let mut textures: Vec<PathBuf> = Vec::new();
        for material in &found.materials {
            for file_name in &material.texture_files {
                if let Some(path) = find_texture(&source_dir, file_name) {
                    push_unique(&mut textures, path);
                }
            }
            for path in textures_named_after(&source_dir, &material.name) {
                push_unique(&mut textures, path);
            }
        }
        for texture in textures {
            let Some(file_name) = texture.file_name() else {
                continue;
            };
            let in_project = if copied {
                let target = target_dir.join(file_name);
                copy_to(&texture, &target)?;
                target
            } else {
                texture
            };
            if self.find_by_source(&in_project).is_none() {
                self.register_texture(&in_project)?;
            }
        }

        let meta = AssetMeta {
            version: ASSET_META_VERSION,
            id: AssetId::new(),
            name,
            source: file_name_of(&source),
            kind: AssetKind::Model(ModelSettings {
                scale: 1.0,
                variants: found
                    .variants
                    .into_iter()
                    .map(|v| VariantInfo {
                        name: v.name,
                        lods: v.lods,
                    })
                    .collect(),
                materials: found
                    .materials
                    .into_iter()
                    .map(|m| MaterialSettings {
                        foliage: outline::looks_like_foliage(&m.name),
                        name: m.name,
                        base_color: None,
                        normal: None,
                    })
                    .collect(),
            }),
        };
        self.register(meta, &target_dir)
    }

    fn register_texture(&mut self, source: &Path) -> Result<AssetId, AssetError> {
        let meta = AssetMeta {
            version: ASSET_META_VERSION,
            id: AssetId::new(),
            name: outline::default_variant_name(source),
            source: file_name_of(source),
            kind: AssetKind::Texture(TextureSettings::default()),
        };
        self.register(meta, source.parent().unwrap_or(Path::new(".")))
    }

    /// `file` as a path below the root (`root/...`), if it lies inside it.
    fn path_in_root(&self, file: &Path) -> Option<PathBuf> {
        let root = fs::canonicalize(self.root()).ok()?;
        let file = fs::canonicalize(file).ok()?;
        file.strip_prefix(&root)
            .ok()
            .map(|relative| self.root().join(relative))
    }
}

fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn push_unique(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.contains(&path) {
        paths.push(path);
    }
}

/// `parent/name`, or `parent/name 2`, ... — the first that does not exist.
fn unused_folder(parent: &Path, name: &str) -> PathBuf {
    let mut folder = parent.join(name);
    let mut copy = 2;
    while folder.exists() {
        folder = parent.join(format!("{name} {copy}"));
        copy += 1;
    }
    folder
}

/// Copy `file` into `folder` (created if needed) and return the new path.
/// With `keep_existing`, a file of the same name already there is not
/// overwritten; the copy gets a numbered name instead.
fn copy_into(file: &Path, folder: &Path, keep_existing: bool) -> Result<PathBuf, AssetError> {
    let mut target = folder.join(file_name_of(file));
    if keep_existing {
        let stem = outline::default_variant_name(file);
        let extension = file
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        let mut copy = 2;
        while target.exists() {
            target = folder.join(format!("{stem} {copy}{extension}"));
            copy += 1;
        }
    }
    copy_to(file, &target)?;
    Ok(target)
}

fn copy_to(from: &Path, to: &Path) -> Result<(), AssetError> {
    if let Some(folder) = to.parent() {
        fs::create_dir_all(folder).map_err(|e| AssetError::Io(folder.to_path_buf(), e))?;
    }
    fs::copy(from, to)
        .map(|_| ())
        .map_err(|e| AssetError::Io(from.to_path_buf(), e))
}

/// Find a texture a material refers to: next to the model or in a
/// `textures` folder below it, also with spaces and underscores swapped
/// (exporters and file names often disagree on those).
fn find_texture(model_dir: &Path, file_name: &str) -> Option<PathBuf> {
    let mut names = vec![file_name.to_string()];
    for variant in [file_name.replace(' ', "_"), file_name.replace('_', " ")] {
        if !names.contains(&variant) {
            names.push(variant);
        }
    }
    [model_dir.to_path_buf(), model_dir.join("textures")]
        .iter()
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|candidate| candidate.is_file())
}

/// Image files next to the model whose name starts with the material's
/// name (minus a `_Mat` / `_Material` suffix) — the loader's fallback for
/// materials that lost their texture connections on export.
fn textures_named_after(model_dir: &Path, material_name: &str) -> Vec<PathBuf> {
    let lower = material_name.to_lowercase();
    let prefix = lower
        .trim_end_matches("_material")
        .trim_end_matches("_mat")
        .trim_end_matches('_')
        .trim();
    if prefix.is_empty() {
        return Vec::new();
    }
    let Ok(entries) = fs::read_dir(model_dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|path| {
            path.is_file()
                && file_kind(path) == Some(FileKind::Texture)
                && file_name_of(path).to_lowercase().starts_with(prefix)
        })
        .collect();
    found.sort();
    found
}

/// Files a model file refers to by relative path and cannot be loaded
/// without: glTF buffers and images, OBJ material libraries.
fn companion_files(model: &Path) -> Vec<PathBuf> {
    let extension = model
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase)
        .unwrap_or_default();
    let references = match extension.as_str() {
        "gltf" => fs::read_to_string(model).map(|text| gltf_uris(&text)),
        "obj" => fs::read_to_string(model).map(|text| obj_material_libraries(&text)),
        _ => return Vec::new(),
    };
    references
        .unwrap_or_default()
        .into_iter()
        .map(PathBuf::from)
        // Only files below the model's folder; nothing absolute and nothing
        // that climbs out of it.
        .filter(|path| {
            path.is_relative()
                && path
                    .components()
                    .all(|c| matches!(c, std::path::Component::Normal(_)))
        })
        .collect()
}

/// The values of all `"uri"` members of a glTF document that name files
/// (data URIs are skipped). A full JSON parser is not needed for this.
fn gltf_uris(text: &str) -> Vec<String> {
    let mut uris = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("\"uri\"") {
        rest = &rest[at + 5..];
        let Some(colon) = rest.find(':') else {
            break;
        };
        // Only whitespace may sit between the key and its colon.
        if !rest[..colon].trim().is_empty() {
            continue;
        }
        let after = rest[colon + 1..].trim_start();
        let Some(value) = after.strip_prefix('"') else {
            continue;
        };
        let Some(end) = value.find('"') else {
            break;
        };
        let uri = &value[..end];
        if !uri.is_empty() && !uri.starts_with("data:") {
            // File names in URIs are percent-encoded; spaces are the
            // common case.
            let uri = uri.replace("%20", " ");
            if !uris.contains(&uri) {
                uris.push(uri);
            }
        }
        rest = &value[end..];
    }
    uris
}

fn obj_material_libraries(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("mtllib "))
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::assets::{database::tests::TempDir, test_models, META_SUFFIX};

    #[test]
    fn finds_file_references_in_gltf_text() {
        let text = r#"{
            "buffers": [{"byteLength": 4, "uri": "pack.bin"}],
            "images": [
                {"uri" : "Bark%20albedo.png"},
                {"uri": "data:image/png;base64,AAAA"},
                {"uri": "pack.bin"}
            ]
        }"#;
        assert_eq!(gltf_uris(text), vec!["pack.bin", "Bark albedo.png"]);
        assert!(gltf_uris("{}").is_empty());
    }

    #[test]
    fn finds_material_libraries_in_obj_text() {
        let text = "# a rock\nmtllib rock.mtl\nv 0 0 0\n";
        assert_eq!(obj_material_libraries(text), vec!["rock.mtl"]);
    }

    #[test]
    fn a_tree_pack_is_split_into_variants_with_lods() {
        let temp = TempDir::new("outline-pack");
        let file = temp.0.join("pack.gltf");
        test_models::write_gltf(
            &file,
            &[
                ("Oak", &["Oak_LOD0", "Oak_LOD1"]),
                ("Pine", &["Pine_LOD0", "Pine_LOD1", "Pine_Billboard"]),
            ],
            None,
        );
        let found = outline::inspect(&file).unwrap();
        let variants: Vec<(&str, Vec<&str>)> = found
            .variants
            .iter()
            .map(|v| (v.name.as_str(), v.lods.iter().map(String::as_str).collect()))
            .collect();
        assert_eq!(
            variants,
            vec![
                ("Oak", vec!["Oak_LOD0", "Oak_LOD1"]),
                ("Pine", vec!["Pine_LOD0", "Pine_LOD1", "Pine_Billboard"]),
            ]
        );
    }

    #[test]
    fn lods_at_the_top_level_are_one_variant_named_after_the_file() {
        let temp = TempDir::new("outline-lods");
        let file = temp.0.join("birch.gltf");
        test_models::write_gltf(&file, &[("birch_LOD0", &[]), ("birch_LOD1", &[])], None);
        let found = outline::inspect(&file).unwrap();
        assert_eq!(found.variants.len(), 1);
        assert_eq!(found.variants[0].name, "birch");
        assert_eq!(found.variants[0].lods, vec!["birch_LOD0", "birch_LOD1"]);
    }

    #[test]
    fn a_plain_model_is_one_variant_with_one_lod() {
        let temp = TempDir::new("outline-plain");
        let file = temp.0.join("rock.gltf");
        test_models::write_gltf(&file, &[("boulder", &[])], None);
        let found = outline::inspect(&file).unwrap();
        assert_eq!(found.variants.len(), 1);
        assert_eq!(found.variants[0].name, "rock");
        assert_eq!(found.variants[0].lods.len(), 1);
    }

    #[test]
    fn importing_from_outside_copies_the_model_and_its_textures() {
        let outside = TempDir::new("import-outside");
        let project = TempDir::new("import-project");
        let file = outside.0.join("Oak tree.gltf");
        test_models::write_gltf(
            &file,
            &[("Oak_LOD0", &[]), ("Oak_LOD1", &[])],
            Some("Bark albedo.png"),
        );
        test_models::write_png(&outside.0.join("Bark albedo.png"));
        // Unrelated files next to the model stay where they are.
        test_models::write_png(&outside.0.join("holiday photo.png"));

        let mut database = AssetDatabase::new(&project.0);
        let id = database.import(&file).unwrap();

        let folder = project.0.join("models").join("Oak tree");
        assert!(folder.join("Oak tree.gltf").is_file());
        assert!(folder.join(format!("Oak tree.gltf{META_SUFFIX}")).is_file());
        assert!(folder.join("Bark albedo.png").is_file());
        assert!(folder.join(format!("Bark albedo.png{META_SUFFIX}")).is_file());
        assert!(!folder.join("holiday photo.png").exists());

        let entry = database.get(id).unwrap();
        assert_eq!(entry.meta.name, "Oak tree");
        let model = entry.meta.as_model().unwrap();
        assert_eq!(model.variants.len(), 1);
        assert_eq!(model.variants[0].lods, vec!["Oak_LOD0", "Oak_LOD1"]);
        // Assimp may append a default material of its own.
        let bark = model.materials.iter().find(|m| m.name == "bark").unwrap();
        assert!(!bark.foliage);
        assert_eq!(bark.base_color, None);
        // The model and its texture.
        assert_eq!(database.len(), 2);

        // The copied model still loads: its texture reference resolves.
        assert!(outline::inspect(&entry.source_path()).is_ok());

        // Importing the original again makes a second copy; importing the
        // copy in the project gives the asset we already have.
        assert_eq!(database.import(&entry.source_path()).unwrap(), id);
        assert_eq!(database.len(), 2);
    }

    #[test]
    fn importing_a_file_inside_the_project_registers_it_in_place() {
        let project = TempDir::new("import-inside");
        let folder = project.0.join("models").join("rocks");
        let file = folder.join("rock.gltf");
        test_models::write_gltf(&file, &[("rock", &[])], None);

        let mut database = AssetDatabase::new(&project.0);
        let id = database.import(&file).unwrap();
        assert_eq!(database.get(id).unwrap().source_path(), file);
        assert!(folder.join(format!("rock.gltf{META_SUFFIX}")).is_file());
        // No copy was made.
        assert_eq!(fs::read_dir(&folder).unwrap().count(), 2);

        // A fresh scan finds the same asset.
        let mut fresh = AssetDatabase::new(&project.0);
        fresh.scan();
        assert_eq!(fresh.get(id).unwrap().meta, database.get(id).unwrap().meta);
    }

    #[test]
    fn importing_a_texture_copies_it_without_overwriting() {
        let outside = TempDir::new("import-texture");
        let project = TempDir::new("import-texture-project");
        let file = outside.0.join("moss.png");
        test_models::write_png(&file);
        fs::create_dir_all(project.0.join("textures")).unwrap();
        fs::write(project.0.join("textures").join("moss.png"), b"someone else's").unwrap();

        let mut database = AssetDatabase::new(&project.0);
        let id = database.import(&file).unwrap();
        let entry = database.get(id).unwrap();
        assert_eq!(entry.source_path(), project.0.join("textures").join("moss 2.png"));
        assert_eq!(
            fs::read(project.0.join("textures").join("moss.png")).unwrap(),
            b"someone else's"
        );
        assert!(matches!(entry.meta.kind, AssetKind::Texture(_)));
    }

    #[test]
    fn unsupported_and_unreadable_files_are_rejected_cleanly() {
        let outside = TempDir::new("import-bad");
        let project = TempDir::new("import-bad-project");
        let mut database = AssetDatabase::new(&project.0);

        let text = outside.0.join("notes.txt");
        fs::write(&text, "hello").unwrap();
        assert!(matches!(database.import(&text), Err(AssetError::Unsupported(_))));

        let missing = outside.0.join("missing.fbx");
        assert!(matches!(database.import(&missing), Err(AssetError::Io(..))));

        let broken = outside.0.join("broken.gltf");
        fs::write(&broken, "this is not a model").unwrap();
        assert!(matches!(database.import(&broken), Err(AssetError::Import(..))));
        // Nothing was copied or registered for any of them.
        assert!(database.is_empty());
        assert!(!project.0.join("models").exists());
    }
}
