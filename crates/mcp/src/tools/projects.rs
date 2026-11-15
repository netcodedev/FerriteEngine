use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::{
    bridge,
    project::{Project, PROJECT_EXTENSION},
    scene::scene_path,
};

use super::*;

pub(super) fn project_info(state: &State) -> Result<Value, String> {
    let p = state.project()?;
    Ok(json!({
        "name": p.file.name,
        "version": p.file.version,
        "startup_scene": p.file.startup_scene,
        "path": p.path.display().to_string(),
        "dir": p.dir().display().to_string(),
        "editor_open": bridge::is_available(p.dir()),
    }))
}

/// A path a tool gave: absolute, or relative to where the server runs.
fn given_path(path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    }
}

pub(super) fn create_project(state: &State, args: Value) -> Result<Value, String> {
    let dir = given_path(&string_field(&args, "path")?);
    if Project::find_in(&dir).is_some() {
        return Err(format!("{} already has a project (open_project opens it)", dir.display()));
    }
    let name = match args.get("name").and_then(Value::as_str) {
        Some(name) if !name.trim().is_empty() => name.trim().to_string(),
        _ => dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "project".into()),
    };
    if name.contains(['/', '\\']) {
        return Err("a project name cannot contain slashes".into());
    }
    for folder in FOLDERS {
        let folder = dir.join(folder);
        std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    }
    let scene = dir.join(STARTER_SCENE_PATH);
    std::fs::write(&scene, STARTER_SCENE).map_err(|e| format!("{}: {e}", scene.display()))?;
    let fp = dir.join(format!("{name}.{PROJECT_EXTENSION}"));
    let text = format!(
        "(\n    version: 1,\n    name: {},\n    startup_scene: Some(\"{STARTER_SCENE_PATH}\"),\n)\n",
        ron::to_string(&name).map_err(|e| e.to_string())?
    );
    std::fs::write(&fp, text).map_err(|e| format!("{}: {e}", fp.display()))?;

    let player_assets = if args.get("player_assets").and_then(Value::as_bool).unwrap_or(true) {
        match copy_player_assets(&dir) {
            Ok(count) => json!({ "copied": count }),
            Err(e) => json!({ "error": e }),
        }
    } else {
        Value::Null
    };
    let open = args.get("open").and_then(Value::as_bool).unwrap_or(true);
    if open {
        state.set_dir(dir.clone());
    }
    Ok(json!({
        "project": fp.display().to_string(),
        "startup_scene": STARTER_SCENE_PATH,
        "player_assets": player_assets,
        "now_working_on": state.dir().display().to_string(),
        "editor": format!("cargo run -p ferrite-editor -- {}", dir.display()),
    }))
}

/// Copy the player prefab's model and animations from the engine's own
/// assets into the project at `dir` (as the engine does for a new project).
fn copy_player_assets(dir: &Path) -> Result<usize, String> {
    copy_player_assets_from(&engine_workspace().join("assets"), dir)
}

/// [`copy_player_assets`] from the assets folder `engine_assets`.
pub(super) fn copy_player_assets_from(engine_assets: &Path, dir: &Path) -> Result<usize, String> {
    let mut files = vec![
        PathBuf::from("models/Mannequin.fbx"),
        PathBuf::from("models/Mannequin.fbx.asset.ron"),
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

pub(super) fn open_project(state: &State, args: Value) -> Result<Value, String> {
    let dir = given_path(&string_field(&args, "path")?);
    let dir = if dir.extension().is_some_and(|e| e == PROJECT_EXTENSION) {
        dir.parent().map(Path::to_path_buf).unwrap_or(dir)
    } else {
        dir
    };
    let project = Project::open_at(&dir)?;
    state.set_dir(project.dir().to_path_buf());
    project_info(state)
}

pub(super) fn set_startup_scene(state: &State, args: Value) -> Result<Value, String> {
    let scene = string_field(&args, "scene")?;
    let mut project = state.project()?;
    let full = scene_path(project.dir(), &scene)?;
    if !full.is_file() {
        return Err(format!("no scene at {}", full.display()));
    }
    let rel = scene
        .trim_start_matches("./")
        .trim_start_matches("scenes/")
        .to_string();
    project.file.startup_scene = Some(format!("scenes/{rel}"));
    project.save()?;
    Ok(json!({"startup_scene": project.file.startup_scene}))
}

pub(super) fn list_scenes(state: &State) -> Result<Value, String> {
    Ok(json!({"scenes": state.project()?.list_scenes()}))
}

pub(super) fn list_assets(state: &State) -> Result<Value, String> {
    Ok(json!({"assets": state.project()?.list_assets()}))
}
