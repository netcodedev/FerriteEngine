//! A project: a folder with a `.fp` file (Ferrite project), the assets,
//! scenes and scripts below it. Engine and editor work inside the project
//! folder: it becomes the working directory, and every path in the project
//! is relative to it.

use std::path::{Path, PathBuf};

use ron::Value;
use serde::{Deserialize, Serialize};

use super::{
    assets,
    scene::scene_file::STARTER_SCENE,
    script::{engine_workspace, ScriptCrate},
};

/// Where a new project's startup scene goes.
pub const STARTER_SCENE_PATH: &str = "scenes/main.ron";

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
    /// Scene a game (played or exported) starts in, e.g. its main menu;
    /// the startup scene when there is none. The editor opens the scene
    /// last edited (`startup_scene`); the game always begins here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_scene: Option<String>,
    /// Scene the dedicated server plays (the world, not the menu); the
    /// game scene when there is none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_scene: Option<String>,
    /// Settings of the editor (its layout), which only it understands.
    #[serde(default = "unit", skip_serializing_if = "is_unit")]
    pub editor: Value,
    /// The project's graphics settings (see `graphics::ProjectGraphics`).
    #[serde(default, skip_serializing_if = "super::graphics::ProjectGraphics::is_default")]
    pub graphics: super::graphics::ProjectGraphics,
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
        // A folder may hold a project's content without its `.fp` file (lost,
        // or not checked in): its main scene is kept, not replaced.
        let scene = dir.join(STARTER_SCENE_PATH);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&scene) {
            Ok(mut file) => {
                use std::io::Write;
                file.write_all(STARTER_SCENE.as_bytes())
                    .map_err(|e| format!("{}: {e}", scene.display()))?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                log::info!("{} is kept as the startup scene", scene.display());
            }
            Err(e) => return Err(format!("{}: {e}", scene.display())),
        }
        match copy_player_assets(dir) {
            Ok(_) => {}
            Err(e) => log::warn!("the player prefab will not work in this project: {e}"),
        }
        let project = Project {
            path: dir.join(format!("{name}.{PROJECT_EXTENSION}")),
            file: ProjectFile {
                version: PROJECT_VERSION,
                name,
                startup_scene: Some(STARTER_SCENE_PATH.to_string()),
                game_scene: None,
                server_scene: None,
                editor: Value::Unit,
                graphics: Default::default(),
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
        super::utils::write_atomically(&self.path, text)
            .map_err(|e| format!("{}: {e}", self.path.display()))
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

    /// The scene a game starts in (see `game_scene`).
    pub fn game_scene(&self) -> Option<String> {
        self.file.game_scene.clone().or_else(|| self.file.startup_scene.clone())
    }

    /// The scene a dedicated server plays (see `server_scene`).
    pub fn server_scene(&self) -> Option<String> {
        self.file.server_scene.clone().or_else(|| self.game_scene())
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
        super::graphics::init_for_project(self.name(), &self.file.graphics);
        Ok(())
    }

    /// The crate the project's scripts are compiled in.
    pub fn scripts(&self) -> ScriptCrate {
        ScriptCrate::new(self.dir())
    }
}

/// Copy what the player prefab needs (its model and animations) from the
/// engine's own `assets` folder into the project at `dir`, keeping files
/// that are already there. Returns how many files were copied.
pub fn copy_player_assets(dir: &Path) -> Result<usize, String> {
    copy_player_assets_from(&engine_workspace().join("assets"), dir)
}

/// [`copy_player_assets`] from the assets folder `engine_assets`.
fn copy_player_assets_from(engine_assets: &Path, dir: &Path) -> Result<usize, String> {
    if !engine_assets.is_dir() {
        return Err(format!("no engine assets at {}", engine_assets.display()));
    }
    let model = crate::player::PLAYER_MODEL_FILE;
    let mut files: Vec<PathBuf> = vec![
        PathBuf::from("models").join(model),
        PathBuf::from("models").join(format!("{model}{}", assets::META_SUFFIX)),
    ];
    let animations = engine_assets.join("animations");
    let entries =
        std::fs::read_dir(&animations).map_err(|e| format!("{}: {e}", animations.display()))?;
    for entry in entries.flatten() {
        if entry.path().is_file() {
            files.push(PathBuf::from("animations").join(entry.file_name()));
        }
    }
    let mut copied = 0;
    for file in files {
        let from = engine_assets.join(&file);
        let to = dir.join("assets").join(&file);
        if to.exists() || !from.is_file() {
            continue;
        }
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::copy(&from, &to).map_err(|e| format!("{}: {e}", from.display()))?;
        copied += 1;
    }
    Ok(copied)
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
        // It opens with a scene to look at.
        assert_eq!(project.file.startup_scene.as_deref(), Some(STARTER_SCENE_PATH));
        assert!(project.dir().join(STARTER_SCENE_PATH).is_file());

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

    #[test]
    fn a_folder_with_scenes_but_no_project_file_keeps_its_main_scene() {
        let dir = std::env::temp_dir().join(format!("ferrite-project-kept-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("scenes")).unwrap();
        std::fs::write(dir.join(STARTER_SCENE_PATH), "(entities: [])").unwrap();
        let project = Project::create(&dir).unwrap();
        assert_eq!(project.file.startup_scene.as_deref(), Some(STARTER_SCENE_PATH));
        assert_eq!(
            std::fs::read_to_string(dir.join(STARTER_SCENE_PATH)).unwrap(),
            "(entities: [])"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The engine's `assets` folder is not in the repository (CI has none),
    /// so this copies from a stand-in for it.
    #[test]
    fn the_player_model_and_animations_are_copied_and_kept() {
        let dir = std::env::temp_dir().join(format!("ferrite-player-assets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let engine_assets = dir.join("engine-assets");
        let project = dir.join("project");
        let model = crate::player::PLAYER_MODEL_FILE;
        let meta = format!("{model}{}", assets::META_SUFFIX);
        std::fs::create_dir_all(engine_assets.join("models")).unwrap();
        std::fs::create_dir_all(engine_assets.join("animations")).unwrap();
        std::fs::write(engine_assets.join("models").join(model), "model").unwrap();
        std::fs::write(engine_assets.join("models").join(&meta), "meta").unwrap();
        std::fs::write(engine_assets.join("models/Other.fbx"), "other").unwrap();
        std::fs::write(engine_assets.join("animations/Idle.fbx"), "idle").unwrap();
        std::fs::write(engine_assets.join("animations/Walk.fbx"), "walk").unwrap();

        assert_eq!(copy_player_assets_from(&engine_assets, &project), Ok(4));
        let copied = |path: &str| std::fs::read_to_string(project.join("assets").join(path)).ok();
        assert_eq!(copied(&format!("models/{model}")).as_deref(), Some("model"));
        assert_eq!(copied(&format!("models/{meta}")).as_deref(), Some("meta"));
        assert_eq!(copied("animations/Idle.fbx").as_deref(), Some("idle"));
        assert_eq!(copied("animations/Walk.fbx").as_deref(), Some("walk"));
        // Only what the player needs.
        assert_eq!(copied("models/Other.fbx"), None);

        // Files already in the project are kept.
        std::fs::write(project.join("assets/animations/Idle.fbx"), "mine").unwrap();
        assert_eq!(copy_player_assets_from(&engine_assets, &project), Ok(0));
        assert_eq!(copied("animations/Idle.fbx").as_deref(), Some("mine"));

        // Without engine assets it says so instead of copying nothing.
        assert!(copy_player_assets_from(&dir.join("nowhere"), &project).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
