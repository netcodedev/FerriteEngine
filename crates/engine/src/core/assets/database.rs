use std::{
    collections::BTreeMap,
    fmt, fs, io,
    path::{Path, PathBuf},
};

use super::{AssetId, AssetMeta, AssetRef, ASSET_META_VERSION, META_SUFFIX};

#[derive(Debug)]
pub enum AssetError {
    Io(PathBuf, io::Error),
    /// The sidecar could not be read or written as RON.
    Format(PathBuf, String),
    /// The file type cannot be imported.
    Unsupported(PathBuf),
    /// The source file could not be understood by its importer.
    Import(PathBuf, String),
    UnknownAsset(AssetId),
}

impl fmt::Display for AssetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AssetError::Io(path, e) => write!(f, "{}: {e}", path.display()),
            AssetError::Format(path, e) => write!(f, "{}: {e}", path.display()),
            AssetError::Unsupported(path) => {
                write!(f, "{}: this kind of file cannot be imported", path.display())
            }
            AssetError::Import(path, e) => write!(f, "{}: {e}", path.display()),
            AssetError::UnknownAsset(id) => write!(f, "asset {id} does not exist"),
        }
    }
}

impl std::error::Error for AssetError {}

/// One registered asset.
#[derive(Clone, Debug)]
pub struct AssetEntry {
    pub meta: AssetMeta,
    /// Folder holding the source file and its sidecar.
    pub dir: PathBuf,
    /// Goes up every time the asset's settings are saved; users of the
    /// asset compare it to notice changes.
    pub revision: u64,
}

impl AssetEntry {
    pub fn source_path(&self) -> PathBuf {
        self.dir.join(&self.meta.source)
    }

    pub fn meta_path(&self) -> PathBuf {
        self.dir.join(format!("{}{META_SUFFIX}", self.meta.source))
    }
}

/// All assets below one root folder. Built by scanning for sidecar files;
/// there is no index on disk.
pub struct AssetDatabase {
    root: PathBuf,
    assets: BTreeMap<AssetId, AssetEntry>,
    next_revision: u64,
    changes: u64,
}

impl AssetDatabase {
    /// An empty database for `root`. Call `scan` to fill it.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            assets: BTreeMap::new(),
            next_revision: 1,
            changes: 0,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Goes up whenever the set of assets or any of their settings may
    /// have changed. Lets views of the database know when to rebuild.
    pub fn changes(&self) -> u64 {
        self.changes
    }

    /// Re-read every sidecar below the root. Assets that are still there
    /// keep their revision unless their sidecar changed. Returns one
    /// message per sidecar that had to be skipped.
    pub fn scan(&mut self) -> Vec<String> {
        let mut problems = Vec::new();
        let mut found: BTreeMap<AssetId, AssetEntry> = BTreeMap::new();
        let mut pending = vec![self.root.clone()];
        while let Some(dir) = pending.pop() {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
            // `read_dir` order depends on the file system.
            paths.sort();
            for path in paths {
                if path.is_dir() {
                    pending.push(path);
                    continue;
                }
                let is_meta = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with(META_SUFFIX));
                if !is_meta {
                    continue;
                }
                match Self::read_meta(&path) {
                    Ok(meta) => {
                        if let Some(other) = found.get(&meta.id) {
                            problems.push(format!(
                                "{}: id {} is already used by {}",
                                path.display(),
                                meta.id,
                                other.meta_path().display()
                            ));
                            continue;
                        }
                        let revision = match self.assets.get(&meta.id) {
                            Some(known) if known.meta == meta && known.dir == dir => known.revision,
                            _ => self.take_revision(),
                        };
                        found.insert(
                            meta.id,
                            AssetEntry {
                                meta,
                                dir: dir.clone(),
                                revision,
                            },
                        );
                    }
                    Err(e) => problems.push(e.to_string()),
                }
            }
        }
        self.assets = found;
        self.changes += 1;
        problems
    }

    fn read_meta(path: &Path) -> Result<AssetMeta, AssetError> {
        let text = fs::read_to_string(path).map_err(|e| AssetError::Io(path.to_path_buf(), e))?;
        let meta = AssetMeta::from_ron(&text)
            .map_err(|e| AssetError::Format(path.to_path_buf(), e.to_string()))?;
        if meta.version > ASSET_META_VERSION {
            return Err(AssetError::Format(
                path.to_path_buf(),
                format!(
                    "written by a newer version (format {}, supported up to {ASSET_META_VERSION})",
                    meta.version
                ),
            ));
        }
        Ok(meta)
    }

    fn take_revision(&mut self) -> u64 {
        let revision = self.next_revision;
        self.next_revision += 1;
        revision
    }

    pub fn get(&self, id: AssetId) -> Option<&AssetEntry> {
        self.assets.get(&id)
    }

    pub fn len(&self) -> usize {
        self.assets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.assets.is_empty()
    }

    /// Every asset, ordered by id.
    pub fn iter(&self) -> impl Iterator<Item = &AssetEntry> {
        self.assets.values()
    }

    /// Name to show for a reference: the asset's name, followed by the part
    /// if the reference names one. `None` if the asset does not exist.
    pub fn display_name(&self, asset: &AssetRef) -> Option<String> {
        let entry = self.get(asset.id)?;
        Some(match &asset.part {
            Some(part) => format!("{} / {part}", entry.meta.name),
            None => entry.meta.name.clone(),
        })
    }

    /// The asset registered for this source file, if any.
    pub fn find_by_source(&self, source: &Path) -> Option<AssetId> {
        let wanted = fs::canonicalize(source).ok()?;
        self.assets
            .values()
            .find(|entry| fs::canonicalize(entry.source_path()).is_ok_and(|p| p == wanted))
            .map(|entry| entry.meta.id)
    }

    /// Register a new asset: write its sidecar into `dir` (next to the
    /// source file named in `meta`) and add it.
    pub fn register(&mut self, meta: AssetMeta, dir: &Path) -> Result<AssetId, AssetError> {
        let id = meta.id;
        let entry = AssetEntry {
            meta,
            dir: dir.to_path_buf(),
            revision: self.take_revision(),
        };
        Self::write_meta(&entry)?;
        self.assets.insert(id, entry);
        self.changes += 1;
        Ok(id)
    }

    /// Replace an asset's settings and save them. Its revision goes up, so
    /// whatever uses the asset reloads it.
    pub fn update(&mut self, meta: AssetMeta) -> Result<(), AssetError> {
        let revision = self.take_revision();
        let entry = self
            .assets
            .get_mut(&meta.id)
            .ok_or(AssetError::UnknownAsset(meta.id))?;
        // The sidecar is named after the source file.
        let previous = entry.meta_path();
        entry.meta = meta;
        entry.revision = revision;
        self.changes += 1;
        Self::write_meta(entry)?;
        if previous != entry.meta_path() {
            let _ = fs::remove_file(previous);
        }
        Ok(())
    }

    fn write_meta(entry: &AssetEntry) -> Result<(), AssetError> {
        let path = entry.meta_path();
        let text = entry
            .meta
            .to_ron()
            .map_err(|e| AssetError::Format(path.clone(), e.to_string()))?;
        fs::write(&path, text).map_err(|e| AssetError::Io(path, e))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::core::assets::{AssetKind, MaterialSettings, ModelSettings, TextureSettings, VariantInfo};

    /// A fresh, empty folder below the system's temp folder, removed again
    /// when the guard is dropped.
    pub(crate) struct TempDir(pub PathBuf);

    impl TempDir {
        pub(crate) fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "ferrite-{label}-{}-{}",
                std::process::id(),
                AssetId::new()
            ));
            fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn texture_meta(name: &str, source: &str) -> AssetMeta {
        AssetMeta {
            version: ASSET_META_VERSION,
            id: AssetId::new(),
            name: name.into(),
            source: source.into(),
            kind: AssetKind::Texture(TextureSettings::default()),
        }
    }

    #[test]
    fn registered_assets_are_found_again_by_a_scan() {
        let temp = TempDir::new("assets-scan");
        let folder = temp.0.join("textures").join("bark");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("bark.png"), b"not really a png").unwrap();

        let mut database = AssetDatabase::new(&temp.0);
        let id = database.register(texture_meta("Bark", "bark.png"), &folder).unwrap();
        assert!(folder.join("bark.png.asset.ron").is_file());

        let mut fresh = AssetDatabase::new(&temp.0);
        assert!(fresh.scan().is_empty());
        assert_eq!(fresh.len(), 1);
        let entry = fresh.get(id).unwrap();
        assert_eq!(entry.meta.name, "Bark");
        assert_eq!(entry.source_path(), folder.join("bark.png"));
        assert_eq!(fresh.find_by_source(&folder.join("bark.png")), Some(id));
        assert_eq!(fresh.find_by_source(&folder.join("missing.png")), None);
    }

    #[test]
    fn model_settings_survive_saving() {
        let temp = TempDir::new("assets-model");
        let texture = AssetId::new();
        let meta = AssetMeta {
            version: ASSET_META_VERSION,
            id: AssetId::new(),
            name: "Tree pack".into(),
            source: "pack.fbx".into(),
            kind: AssetKind::Model(ModelSettings {
                scale: 0.5,
                variants: vec![VariantInfo {
                    name: "Oak".into(),
                    lods: vec!["Oak_LOD0".into(), "Oak_LOD1".into()],
                }],
                materials: vec![MaterialSettings {
                    name: "Bark".into(),
                    base_color: Some(texture),
                    foliage: false,
                    ..Default::default()
                }],
                clips: Vec::new(),
            }),
        };
        let mut database = AssetDatabase::new(&temp.0);
        let id = database.register(meta.clone(), &temp.0).unwrap();

        let mut fresh = AssetDatabase::new(&temp.0);
        fresh.scan();
        assert_eq!(fresh.get(id).unwrap().meta, meta);
    }

    #[test]
    fn updating_bumps_the_revision_and_rescanning_keeps_it() {
        let temp = TempDir::new("assets-revision");
        let mut database = AssetDatabase::new(&temp.0);
        let id = database.register(texture_meta("Bark", "bark.png"), &temp.0).unwrap();
        let first = database.get(id).unwrap().revision;

        // Nothing changed on disk: same revision.
        database.scan();
        assert_eq!(database.get(id).unwrap().revision, first);

        let mut meta = database.get(id).unwrap().meta.clone();
        meta.name = "Old bark".into();
        database.update(meta).unwrap();
        let second = database.get(id).unwrap().revision;
        assert!(second > first);

        // Changed by someone else on disk: noticed by the next scan.
        let entry = database.get(id).unwrap().clone();
        let mut edited = entry.meta.clone();
        edited.name = "Edited outside".into();
        fs::write(entry.meta_path(), edited.to_ron().unwrap()).unwrap();
        database.scan();
        assert!(database.get(id).unwrap().revision > second);
        assert_eq!(database.get(id).unwrap().meta.name, "Edited outside");
    }

    #[test]
    fn broken_and_duplicate_sidecars_are_reported_not_fatal() {
        let temp = TempDir::new("assets-problems");
        let mut database = AssetDatabase::new(&temp.0);
        let meta = texture_meta("Bark", "bark.png");
        database.register(meta.clone(), &temp.0).unwrap();
        fs::write(temp.0.join("broken.png.asset.ron"), "this is not ron (").unwrap();
        let copy = temp.0.join("copy");
        fs::create_dir_all(&copy).unwrap();
        fs::write(copy.join("bark.png.asset.ron"), meta.to_ron().unwrap()).unwrap();

        let mut fresh = AssetDatabase::new(&temp.0);
        let problems = fresh.scan();
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert_eq!(fresh.len(), 1);
    }

    #[test]
    fn update_of_an_unknown_asset_fails() {
        let temp = TempDir::new("assets-unknown");
        let mut database = AssetDatabase::new(&temp.0);
        let result = database.update(texture_meta("Ghost", "ghost.png"));
        assert!(matches!(result, Err(AssetError::UnknownAsset(_))));
    }
}
