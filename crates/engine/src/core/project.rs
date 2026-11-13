//! A project: a folder with a `.fp` file (Ferrite project), the assets,
//! scenes and scripts below it. Engine and editor work inside the project
//! folder: it becomes the working directory, and every path in the project
//! is relative to it.

use std::path::{Path, PathBuf};

use ron::Value;
use serde::{Deserialize, Serialize};

use super::{assets, script::ScriptCrate};

/// Extension of a project file.
pub const PROJECT_EXTENSION: &str = "fp";
const PROJECT_VERSION: u32 = 1;

/// Folders every project has.
pub const FOLDERS: [&str; 6] = [
    "assets/models",
    "assets/textures",
    "assets/animations",
    "animations",
    "scenes",
    "scripts",
];

/// What a `.fp` file holds.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ProjectFile {
    pub version: u32,
    pub name: String,
    /// Scene the project opens with, relative to the project folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_scene: Option<String>,
    /// Settings of the editor (its layout), which only it understands.
    #[serde(default = "unit", skip_serializing_if = "is_unit")]
    pub editor: Value,
}

fn unit() -> Value {
    Value::Unit
}

fn is_unit(value: &Value) -> bool {
    *value == Value::Unit
}

#[derive(Clone, Debug)]
pub struct Project {
    /// The `.fp` file.
    path: PathBuf,
    pub file: ProjectFile,
}

impl Project {
    /// Make a new, empty project named after `dir` in `dir` (created if
    /// needed): its project file and folders. An existing project there is
    /// opened instead.
    pub fn create(dir: &Path) -> Result<Project, String> {
        if let Some(existing) = Self::find_in(dir) {
            return Self::load(&existing);
        }
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "project".to_string());
        for folder in FOLDERS {
            let folder = dir.join(folder);
            std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
        }
        let project = Project {
            path: dir.join(format!("{name}.{PROJECT_EXTENSION}")),
            file: ProjectFile {
                version: PROJECT_VERSION,
                name,
                startup_scene: None,
                editor: Value::Unit,
            },
        };
        project.save()?;
        Ok(project)
    }

    pub fn load(path: &Path) -> Result<Project, String> {
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
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

    /// The first `.fp` file in `dir`.
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

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn dir(&self) -> &Path {
        self.path.parent().unwrap_or(Path::new("."))
    }

    pub fn name(&self) -> &str {
        &self.file.name
    }

    /// Work in this project from now on: it becomes the working directory
    /// and its assets are scanned.
    pub fn enter(&self) -> Result<(), String> {
        std::env::set_current_dir(self.dir())
            .map_err(|e| format!("{}: {e}", self.dir().display()))?;
        for folder in FOLDERS {
            let _ = std::fs::create_dir_all(folder);
        }
        assets::reopen();
        Ok(())
    }

    /// The crate the project's scripts are compiled in.
    pub fn scripts(&self) -> ScriptCrate {
        ScriptCrate::new(self.dir())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_project_has_its_file_and_folders_and_reopens() {
        let dir = std::env::temp_dir().join(format!("ferrite-project-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let project = Project::create(&dir.join("My Game")).unwrap();
        assert_eq!(project.name(), "My Game");
        assert!(project.path().ends_with("My Game/My Game.fp"));
        for folder in FOLDERS {
            assert!(project.dir().join(folder).is_dir(), "{folder}");
        }

        let mut changed = project.clone();
        changed.file.startup_scene = Some("scenes/main.ron".into());
        changed.save().unwrap();
        let found = Project::find_in(project.dir()).unwrap();
        let loaded = Project::load(&found).unwrap();
        assert_eq!(loaded.file, changed.file);
        // Creating it again opens what is there.
        assert_eq!(Project::create(project.dir()).unwrap().file, changed.file);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
