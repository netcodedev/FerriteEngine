//! Read and write the Ferrite project file (`.fp`) and walk the folders a
//! project has (assets, scenes, scripts) — without linking the engine.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Extension of a project file.
pub const PROJECT_EXTENSION: &str = "fp";

/// The subset of the engine's `ProjectFile` we need on disk. The rest of
/// the file (the editor's own layout) is kept in `editor` as a raw RON
/// value and round-tripped unchanged.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ProjectFile {
    pub version: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_scene: Option<String>,
    /// Kept as they are (see the engine's `ProjectFile`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_scene: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_scene: Option<String>,
    #[serde(default = "unit_value", skip_serializing_if = "is_unit")]
    pub editor: ron::Value,
    /// The engine's graphics section, kept as it is.
    #[serde(default = "unit_value", skip_serializing_if = "is_unit")]
    pub graphics: ron::Value,
}

fn unit_value() -> ron::Value {
    ron::Value::Unit
}

fn is_unit(value: &ron::Value) -> bool {
    *value == ron::Value::Unit
}

/// A project: the `.fp` file plus the folder it lives in.
#[derive(Clone, Debug)]
pub struct Project {
    pub path: PathBuf,
    pub file: ProjectFile,
}

impl Project {
    pub fn load(path: &Path) -> Result<Project, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let file: ProjectFile =
            ron::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        Ok(Project { path, file })
    }

    pub fn save(&self) -> Result<(), String> {
        let text = ron::ser::to_string_pretty(&self.file, ron::ser::PrettyConfig::default())
            .map_err(|e| e.to_string())?;
        std::fs::write(&self.path, text).map_err(|e| format!("{}: {e}", self.path.display()))
    }

    /// The first `.fp` file in `dir` (same rule the engine uses).
    pub fn find_in(dir: &Path) -> Option<PathBuf> {
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == PROJECT_EXTENSION))
            .collect();
        files.sort();
        files.into_iter().next()
    }

    /// Open the project rooted at `dir` (looks for its `.fp` file).
    pub fn open_at(dir: &Path) -> Result<Project, String> {
        let fp = Self::find_in(dir)
            .ok_or_else(|| format!("no .fp file in {}", dir.display()))?;
        Self::load(&fp)
    }

    pub fn dir(&self) -> &Path {
        self.path.parent().unwrap_or(Path::new("."))
    }

    /// Scenes under `scenes/` (relative paths with forward slashes), sorted.
    pub fn list_scenes(&self) -> Vec<String> {
        list_ron_files(&self.dir().join("scenes"), "scenes")
    }

    /// Scripts under `scripts/` (relative paths).
    pub fn list_scripts(&self) -> Vec<String> {
        list_files(&self.dir().join("scripts"), "scripts", &["rs"])
    }

    /// Assets under `assets/` (relative paths), bucketed by top-level folder.
    pub fn list_assets(&self) -> Vec<String> {
        let mut out = Vec::new();
        let root = self.dir().join("assets");
        if root.is_dir() {
            walk(&root, &self.dir().join("assets"), "assets", &mut out);
        }
        out.sort();
        out
    }
}

fn list_ron_files(dir: &Path, prefix: &str) -> Vec<String> {
    list_files(dir, prefix, &["ron"])
}

fn list_files(dir: &Path, prefix: &str, extensions: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    if dir.is_dir() {
        walk_matching(dir, dir, prefix, extensions, &mut out);
    }
    out.sort();
    out
}

fn walk_matching(
    base: &Path,
    dir: &Path,
    prefix: &str,
    extensions: &[&str],
    out: &mut Vec<String>,
) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk_matching(base, &p, prefix, extensions, out);
            } else if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
                if extensions.iter().any(|w| w.eq_ignore_ascii_case(ext)) {
                    if let Some(rel) = p.strip_prefix(base).ok().and_then(|r| r.to_str()) {
                        out.push(format!("{prefix}/{}", rel.replace('\\', "/")));
                    }
                }
            }
        }
    }
}

fn walk(base: &Path, dir: &Path, prefix: &str, out: &mut Vec<String>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(base, &p, prefix, out);
            } else if let Some(rel) = p.strip_prefix(base).ok().and_then(|r| r.to_str()) {
                out.push(format!("{prefix}/{}", rel.replace('\\', "/")));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ferrite-mcp-{}-{}-{}",
            name,
            std::process::id(),
            uniq()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn uniq() -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(1);
        N.fetch_add(1, Ordering::Relaxed)
    }

    fn write_project(dir: &Path, name: &str, startup: Option<&str>) {
        let content = if let Some(s) = startup {
            format!(
                "(version:1,name:\"{name}\",startup_scene:Some(\"{s}\"),editor:())"
            )
        } else {
            format!("(version:1,name:\"{name}\",editor:())")
        };
        std::fs::write(dir.join(format!("{name}.fp")), content).unwrap();
    }

    #[test]
    fn open_at_reads_the_project_file_and_its_fields() {
        let dir = tmp("open");
        write_project(&dir, "MyGame", Some("scenes/main.ron"));
        let project = Project::open_at(&dir).unwrap();
        assert_eq!(project.file.name, "MyGame");
        assert_eq!(project.file.version, 1);
        assert_eq!(
            project.file.startup_scene.as_deref(),
            Some("scenes/main.ron")
        );
    }

    #[test]
    fn find_in_picks_the_first_fp_file_in_sorted_order() {
        let dir = tmp("find");
        std::fs::write(dir.join("b.fp"), "(version:1,name:\"b\",editor:())").unwrap();
        std::fs::write(dir.join("a.fp"), "(version:1,name:\"a\",editor:())").unwrap();
        let found = Project::find_in(&dir).unwrap();
        assert!(found.ends_with("a.fp"));
    }

    #[test]
    fn open_at_fails_when_no_fp_file_is_present() {
        let dir = tmp("none");
        assert!(Project::open_at(&dir).is_err());
    }

    #[test]
    fn save_roundtrips_the_project_file() {
        let dir = tmp("save");
        write_project(&dir, "MyGame", None);
        let mut project = Project::open_at(&dir).unwrap();
        project.file.startup_scene = Some("scenes/level1.ron".into());
        project.save().unwrap();
        let reloaded = Project::open_at(&dir).unwrap();
        assert_eq!(
            reloaded.file.startup_scene.as_deref(),
            Some("scenes/level1.ron")
        );
    }

    #[test]
    fn list_scenes_returns_ron_files_under_scenes() {
        let dir = tmp("list-scenes");
        write_project(&dir, "G", None);
        std::fs::create_dir_all(dir.join("scenes/subfolder")).unwrap();
        std::fs::write(dir.join("scenes/one.ron"), "()").unwrap();
        std::fs::write(dir.join("scenes/two.ron"), "()").unwrap();
        std::fs::write(dir.join("scenes/subfolder/three.ron"), "()").unwrap();
        std::fs::write(dir.join("scenes/not-a-scene.txt"), "()").unwrap();

        let project = Project::open_at(&dir).unwrap();
        let scenes = project.list_scenes();
        assert!(scenes.contains(&"scenes/one.ron".to_string()));
        assert!(scenes.contains(&"scenes/two.ron".to_string()));
        assert!(scenes.contains(&"scenes/subfolder/three.ron".to_string()));
        assert_eq!(scenes.len(), 3);
    }

    #[test]
    fn list_scripts_walks_only_rust_files() {
        let dir = tmp("list-scripts");
        write_project(&dir, "G", None);
        std::fs::create_dir_all(dir.join("scripts")).unwrap();
        std::fs::write(dir.join("scripts/walk.rs"), "// script").unwrap();
        std::fs::write(dir.join("scripts/README.md"), "# readme").unwrap();
        let project = Project::open_at(&dir).unwrap();
        let scripts = project.list_scripts();
        assert_eq!(scripts, vec!["scripts/walk.rs"]);
    }

    #[test]
    fn list_assets_walks_every_file_under_assets() {
        let dir = tmp("list-assets");
        write_project(&dir, "G", None);
        std::fs::create_dir_all(dir.join("assets/models")).unwrap();
        std::fs::create_dir_all(dir.join("assets/textures/grass")).unwrap();
        std::fs::write(dir.join("assets/models/cube.obj"), "x").unwrap();
        std::fs::write(dir.join("assets/textures/grass/albedo.png"), "x").unwrap();
        let project = Project::open_at(&dir).unwrap();
        let assets = project.list_assets();
        assert_eq!(
            assets,
            vec![
                "assets/models/cube.obj".to_string(),
                "assets/textures/grass/albedo.png".to_string(),
            ]
        );
    }
}
