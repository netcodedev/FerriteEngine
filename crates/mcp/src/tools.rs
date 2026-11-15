//! The MCP tools this server offers.
//!
//! Two kinds: tools that work on the project's files (any time, also
//! without an editor), and `editor_*` tools that change the scene open in
//! a running editor through its bridge (see `bridge`). While the editor
//! has a scene open, the file tools refuse to change that scene's file:
//! the editor would overwrite the change with its next save.
//!
//! `catalog()` is what `tools/list` answers; `dispatch` is what
//! `tools/call` runs.

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::{json, Value};

use crate::{
    bridge,
    project::{Project, PROJECT_EXTENSION},
    scene::{component_label, fresh_id, scene_path, EntityData, SceneFile},
};

/// The scene a new project starts with; the engine writes the same one.
pub const STARTER_SCENE: &str = include_str!("../../engine/src/core/scene/starter_scene.ron");
const STARTER_SCENE_PATH: &str = "scenes/main.ron";

/// Folders every project has (as the engine's `project::FOLDERS`).
const FOLDERS: [&str; 6] = [
    "assets/models",
    "assets/textures",
    "assets/animations",
    "animations",
    "scenes",
    "scripts",
];

/// The engine's workspace: where this server was built from.
fn engine_workspace() -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    path.canonicalize().unwrap_or(path)
}

/// Shared state for a running server: the project it works on, which
/// `create_project` and `open_project` can change.
pub struct State {
    project_dir: RefCell<PathBuf>,
}

impl State {
    pub fn new(project_dir: PathBuf) -> Self {
        State {
            project_dir: RefCell::new(project_dir),
        }
    }

    pub fn dir(&self) -> PathBuf {
        self.project_dir.borrow().clone()
    }

    fn set_dir(&self, dir: PathBuf) {
        *self.project_dir.borrow_mut() = dir;
    }

    fn project(&self) -> Result<Project, String> {
        Project::open_at(&self.dir())
    }
}

/// One tool definition for `tools/list`.
pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
}

fn tool(name: &'static str, description: &'static str, properties: Value, required: &[&str]) -> Tool {
    Tool {
        name,
        description,
        input_schema: json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        }),
    }
}

fn vec3() -> Value {
    json!({"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3})
}

fn quat() -> Value {
    json!({"type": "array", "items": {"type": "number"}, "minItems": 4, "maxItems": 4,
           "description": "Quaternion [w, x, y, z]"})
}

fn scene_arg() -> Value {
    json!({"type": "string", "description": "Scene file, e.g. main.ron or scenes/main.ron"})
}

/// Every tool the server offers.
pub fn catalog() -> Vec<Tool> {
    vec![
        // -- Projects -----------------------------------------------------
        tool(
            "project_info",
            "Read the project's .fp file (name, version, startup scene) and whether an editor has it open.",
            json!({}),
            &[],
        ),
        tool(
            "create_project",
            "Make a new project: folders, .fp file, a starter scene (free-flying camera, sky, \
             terrain) set as startup scene, and the player prefab's assets. Afterwards this \
             server works on the new project (unless open is false).",
            json!({
                "path": {"type": "string", "description": "Folder for the project (created if needed)"},
                "name": {"type": "string", "description": "Defaults to the folder's name"},
                "player_assets": {"type": "boolean", "description": "Copy the player model and animations (default true, ~37 MB)"},
                "open": {"type": "boolean", "description": "Work on the new project from now on (default true)"},
            }),
            &["path"],
        ),
        tool(
            "open_project",
            "Work on another project from now on (a folder with a .fp file).",
            json!({"path": {"type": "string"}}),
            &["path"],
        ),
        tool(
            "set_startup_scene",
            "Set the scene the project opens with.",
            json!({"scene": scene_arg()}),
            &["scene"],
        ),
        tool("list_scenes", "List the scene files under scenes/.", json!({}), &[]),
        tool("list_assets", "List every file under assets/.", json!({}), &[]),
        // -- Scripts ------------------------------------------------------
        tool("list_scripts", "List the Rust scripts under scripts/.", json!({}), &[]),
        tool(
            "read_script",
            "Return the source of scripts/<name>.rs.",
            json!({"name": {"type": "string"}}),
            &["name"],
        ),
        tool(
            "write_script",
            "Write scripts/<name>.rs (name in snake_case). A script is a struct with \
             #[derive(Script, Default)] and an impl Behaviour; `use ferrite::prelude::*;`. A \
             running editor rebuilds and reloads scripts when files change (see \
             editor_status.scripts_message); build_scripts shows every compiler error.",
            json!({
                "name": {"type": "string"},
                "source": {"type": "string"},
                "overwrite": {"type": "boolean", "description": "Replace an existing file (default false)"},
            }),
            &["name", "source"],
        ),
        tool(
            "build_scripts",
            "Compile the project's scripts and return the compiler's errors and warnings. Does \
             not load them into an editor (editor_reload_scripts does).",
            json!({}),
            &[],
        ),
        // -- Scene files --------------------------------------------------
        tool(
            "read_scene",
            "Return the raw RON text of a scene file.",
            json!({"scene": scene_arg()}),
            &["scene"],
        ),
        tool(
            "scene_summary",
            "Parse a scene file: entity tree with ids, names, transforms and component labels.",
            json!({"scene": scene_arg()}),
            &["scene"],
        ),
        tool(
            "create_scene",
            "Make a scene file. Empty, or the starter scene (camera, sky, terrain) with starter: true.",
            json!({"scene": scene_arg(), "starter": {"type": "boolean"}}),
            &["scene"],
        ),
        tool(
            "add_entity",
            "Add an entity to a scene file (refused while the editor has that scene open: use \
             editor_add_entity). Returns the new id.",
            json!({
                "scene": scene_arg(), "name": {"type": "string"},
                "position": vec3(), "rotation": quat(), "scale": vec3(),
                "parent_id": {"type": "integer"},
            }),
            &["scene", "name"],
        ),
        tool(
            "remove_entity",
            "Remove an entity and its children from a scene file.",
            json!({"scene": scene_arg(), "id": {"type": "integer"}}),
            &["scene", "id"],
        ),
        tool(
            "move_entity",
            "Set position, rotation or scale of an entity in a scene file; what is left out stays.",
            json!({
                "scene": scene_arg(), "id": {"type": "integer"},
                "position": vec3(), "rotation": quat(), "scale": vec3(),
            }),
            &["scene", "id"],
        ),
        tool(
            "rename_entity",
            "Rename an entity in a scene file.",
            json!({"scene": scene_arg(), "id": {"type": "integer"}, "name": {"type": "string"}}),
            &["scene", "id", "name"],
        ),
        tool(
            "add_component",
            "Add a component to an entity in a scene file, as RON, e.g. \
             Shape(kind: Cube, color: (1.0, 0.6, 0.2)). editor_component_catalog lists every \
             kind with its RON.",
            json!({"scene": scene_arg(), "id": {"type": "integer"}, "ron": {"type": "string"}}),
            &["scene", "id", "ron"],
        ),
        tool(
            "remove_component",
            "Remove the component at index from an entity in a scene file.",
            json!({"scene": scene_arg(), "id": {"type": "integer"}, "index": {"type": "integer"}}),
            &["scene", "id", "index"],
        ),
        // -- Live editor --------------------------------------------------
        tool(
            "editor_status",
            "The running editor: edit or play mode, open scene, unsaved changes, selection, the \
             last status message, the last script build message, loaded scripts, components \
             that could not be built, whether the scene has a camera.",
            json!({}),
            &[],
        ),
        tool(
            "editor_list_entities",
            "Live entity tree: ids, names, transforms, components with their properties.",
            json!({}),
            &[],
        ),
        tool(
            "editor_get_entity",
            "One live entity with its components and their properties.",
            json!({"id": {"type": "integer"}}),
            &["id"],
        ),
        tool(
            "editor_add_entity",
            "Add an entity to the open scene. Returns its id.",
            json!({
                "name": {"type": "string"}, "position": vec3(), "rotation": quat(),
                "scale": vec3(), "parent_id": {"type": "integer"},
            }),
            &["name"],
        ),
        tool(
            "editor_remove_entity",
            "Remove a live entity and its children.",
            json!({"id": {"type": "integer"}}),
            &["id"],
        ),
        tool(
            "editor_move_entity",
            "Set position, rotation or scale of a live entity; what is left out stays.",
            json!({"id": {"type": "integer"}, "position": vec3(), "rotation": quat(), "scale": vec3()}),
            &["id"],
        ),
        tool(
            "editor_rename_entity",
            "Rename a live entity.",
            json!({"id": {"type": "integer"}, "name": {"type": "string"}}),
            &["id", "name"],
        ),
        tool(
            "editor_select",
            "Select an entity in the editor (the inspector shows it); id null clears.",
            json!({"id": {"type": ["integer", "null"]}}),
            &[],
        ),
        tool(
            "editor_component_catalog",
            "Every component kind that can be added, with ready-to-use RON, and every loaded \
             script with its parameters.",
            json!({}),
            &[],
        ),
        tool(
            "editor_add_component",
            "Add a component to a live entity, given as RON (see editor_component_catalog).",
            json!({"id": {"type": "integer"}, "ron": {"type": "string"}}),
            &["id", "ron"],
        ),
        tool(
            "editor_remove_component",
            "Remove the component at index (see editor_get_entity) from a live entity.",
            json!({"id": {"type": "integer"}, "index": {"type": "integer"}}),
            &["id", "index"],
        ),
        tool(
            "editor_set_property",
            "Set a property of a component of a live entity: script parameters, sky light time \
             of day, camera speed, shape colour, ... Names as editor_get_entity shows them \
             (\"time of day\"; \"time_of_day\" works too). The value is converted to the \
             property's kind. The answer has the value the component ended up with; an error \
             if it ignored the change.",
            json!({
                "id": {"type": "integer"},
                "component_index": {"type": "integer"},
                "name": {"type": "string"},
                "value": {"type": ["number", "boolean", "string"]},
            }),
            &["id", "component_index", "name", "value"],
        ),
        tool(
            "editor_open_scene",
            "Open a scene file in the editor. Refused with unsaved changes unless discard_changes.",
            json!({"scene": scene_arg(), "discard_changes": {"type": "boolean"}}),
            &["scene"],
        ),
        tool(
            "editor_reload_scripts",
            "Build the scripts and load them into the editor. Check editor_status.scripts_message after a moment.",
            json!({}),
            &[],
        ),
        tool("editor_play", "Enter play mode.", json!({}), &[]),
        tool("editor_stop", "Leave play mode (the scene goes back to how it was).", json!({}), &[]),
        tool("editor_save", "Save the open scene to its file.", json!({}), &[]),
        tool(
            "editor_focus",
            "Point the editor camera at an entity (edit mode).",
            json!({"id": {"type": "integer"}}),
            &["id"],
        ),
        tool(
            "editor_set_view",
            "Move or turn the editor camera (edit mode); what is left out stays. Yaw 0 looks \
             along +x, 90 along +z; pitch below 0 looks down.",
            json!({"position": vec3(), "yaw_deg": {"type": "number"}, "pitch_deg": {"type": "number"}}),
            &[],
        ),
        tool(
            "viewport_screenshot",
            "What the editor shows, as an image: the viewport (default) or the whole window. \
             Scaled to max_width pixels across (default 1280; 0 for full size).",
            json!({
                "source": {"type": "string", "enum": ["viewport", "window"]},
                "max_width": {"type": "integer"},
                "format": {"type": "string", "enum": ["png", "jpeg"]},
            }),
            &[],
        ),
        // -- Several at once ----------------------------------------------
        tool(
            "batch",
            "Run several tools in one call, in order: operations is a list of {tool, \
             arguments}. Stops at the first error unless stop_on_error is false. Returns each \
             result.",
            json!({
                "operations": {"type": "array", "items": {
                    "type": "object",
                    "properties": {"tool": {"type": "string"}, "arguments": {"type": "object"}},
                    "required": ["tool"],
                }},
                "stop_on_error": {"type": "boolean"},
            }),
            &["operations"],
        ),
    ]
}

pub fn dispatch(state: &State, name: &str, args: Value) -> Result<Value, String> {
    match name {
        "project_info" => project_info(state),
        "create_project" => create_project(state, args),
        "open_project" => open_project(state, args),
        "set_startup_scene" => set_startup_scene(state, args),
        "list_scenes" => list_scenes(state),
        "list_assets" => list_assets(state),
        "list_scripts" => list_scripts(state),
        "read_script" => read_script(state, args),
        "write_script" => write_script(state, args),
        "build_scripts" => build_scripts(state),
        "read_scene" => read_scene(state, args),
        "scene_summary" => scene_summary(state, args),
        "create_scene" => create_scene(state, args),
        "add_entity" => add_entity(state, args),
        "remove_entity" => remove_entity(state, args),
        "move_entity" => move_entity(state, args),
        "rename_entity" => rename_entity(state, args),
        "add_component" => add_component(state, args),
        "remove_component" => remove_component(state, args),
        "editor_status" => editor_call(state, json!({"op": "status"})),
        "editor_list_entities" => editor_call(state, json!({"op": "list_entities"})),
        "editor_get_entity" => editor_pass(state, args, "get_entity"),
        "editor_add_entity" => editor_pass(state, args, "add_entity"),
        "editor_remove_entity" => editor_pass(state, args, "remove_entity"),
        "editor_move_entity" => editor_pass(state, args, "move_entity"),
        "editor_rename_entity" => editor_pass(state, args, "rename_entity"),
        "editor_select" => editor_pass(state, args, "select"),
        "editor_component_catalog" => editor_call(state, json!({"op": "component_catalog"})),
        "editor_add_component" => editor_pass(state, args, "add_component"),
        "editor_remove_component" => editor_pass(state, args, "remove_component"),
        "editor_set_property" => editor_set_property(state, args),
        "editor_open_scene" => editor_open_scene(state, args),
        "editor_reload_scripts" => editor_call(state, json!({"op": "reload_scripts"})),
        "editor_play" => editor_call(state, json!({"op": "play"})),
        "editor_stop" => editor_call(state, json!({"op": "stop"})),
        "editor_save" => editor_call(state, json!({"op": "save_scene"})),
        "editor_focus" => editor_pass(state, args, "focus"),
        "editor_set_view" => editor_pass(state, args, "set_view"),
        "viewport_screenshot" => viewport_screenshot(state, args),
        "batch" => batch(state, args),
        other => Err(format!("unknown tool: {other}")),
    }
}

// ---- projects -----------------------------------------------------------

fn project_info(state: &State) -> Result<Value, String> {
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

fn create_project(state: &State, args: Value) -> Result<Value, String> {
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
    let engine_assets = engine_workspace().join("assets");
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

fn open_project(state: &State, args: Value) -> Result<Value, String> {
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

fn set_startup_scene(state: &State, args: Value) -> Result<Value, String> {
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

fn list_scenes(state: &State) -> Result<Value, String> {
    Ok(json!({"scenes": state.project()?.list_scenes()}))
}

fn list_assets(state: &State) -> Result<Value, String> {
    Ok(json!({"assets": state.project()?.list_assets()}))
}

// ---- scripts ------------------------------------------------------------

fn list_scripts(state: &State) -> Result<Value, String> {
    Ok(json!({"scripts": state.project()?.list_scripts()}))
}

/// `scripts/<name>.rs` of the project. The file name is the module name,
/// so it has to be snake_case.
fn script_path(state: &State, name: &str) -> Result<PathBuf, String> {
    let name = name.trim_end_matches(".rs");
    let valid = name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !valid {
        return Err(format!(
            "'{name}' is not a script file name: lowercase letters, digits and _ (e.g. door_opener)"
        ));
    }
    Ok(state.project()?.dir().join("scripts").join(format!("{name}.rs")))
}

fn read_script(state: &State, args: Value) -> Result<Value, String> {
    let path = script_path(state, &string_field(&args, "name")?)?;
    let source = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(json!({"path": path.display().to_string(), "source": source}))
}

fn write_script(state: &State, args: Value) -> Result<Value, String> {
    let path = script_path(state, &string_field(&args, "name")?)?;
    let source = string_field(&args, "source")?;
    let overwrite = args.get("overwrite").and_then(Value::as_bool).unwrap_or(false);
    if path.exists() && !overwrite {
        return Err(format!("{} exists; pass overwrite: true to replace it", path.display()));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::write(&path, source).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(json!({
        "written": path.display().to_string(),
        "next": "a running editor rebuilds on its own (editor_status → scripts_message); \
                 build_scripts shows all compiler errors",
    }))
}

/// Whether `name` is a variable `cargo run` sets for the program it runs.
/// Passed on to a nested cargo, some make dependencies rebuild (and with
/// them the engine library a running editor has loaded).
fn is_inherited_cargo_var(name: &str) -> bool {
    matches!(
        name,
        "CARGO_MANIFEST_DIR"
            | "CARGO_MANIFEST_PATH"
            | "CARGO_MANIFEST_LINKS"
            | "CARGO_CRATE_NAME"
            | "CARGO_BIN_NAME"
            | "CARGO_PRIMARY_PACKAGE"
            | "CARGO_TARGET_TMPDIR"
            | "CARGO_RUSTC_CURRENT_DIR"
            | "OUT_DIR"
    ) || name.starts_with("CARGO_PKG_")
        || name.starts_with("CARGO_CFG_")
        || name.starts_with("CARGO_BIN_EXE_")
}

/// The diagnostics in cargo's short output, with script paths shown
/// relative to the project.
fn diagnostics(stderr: &str, scripts_dir: &Path) -> (Vec<String>, Vec<String>) {
    let prefix = format!("{}/", scripts_dir.display());
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    for line in stderr.lines() {
        let line = line.trim().replace(&prefix, "scripts/");
        // Cargo's summary lines repeat what the diagnostics said.
        if line.starts_with("error: could not compile") || line.starts_with("warning: `") {
            continue;
        }
        if line.contains("error") {
            errors.push(line);
        } else if line.contains("warning") && line.contains("scripts/") {
            warnings.push(line);
        }
    }
    (errors, warnings)
}

fn build_scripts(state: &State) -> Result<Value, String> {
    let scripts = state.project()?.dir().join("scripts");
    let scripts = scripts.canonicalize().unwrap_or(scripts);
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut command = Command::new(cargo);
    for (name, _) in std::env::vars_os() {
        if name.to_str().is_some_and(is_inherited_cargo_var) {
            command.env_remove(name);
        }
    }
    let output = command
        .current_dir(engine_workspace())
        .args(["build", "-p", "ferrite_scripts", "--message-format=short"])
        .env("FERRITE_SCRIPTS", &scripts)
        .output()
        .map_err(|e| format!("could not run cargo: {e}"))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let (errors, warnings) = diagnostics(&stderr, &scripts);
    if output.status.success() {
        Ok(json!({"ok": true, "warnings": warnings}))
    } else {
        Err(serde_json::to_string_pretty(&json!({
            "ok": false,
            "errors": if errors.is_empty() { vec![stderr.trim().to_string()] } else { errors },
            "warnings": warnings,
        }))
        .unwrap_or_default())
    }
}

// ---- scene files --------------------------------------------------------

/// Refuse to change the file of the scene a running editor has open: its
/// next save would write its own state over the change.
fn guard_open_scene(state: &State, path: &Path) -> Result<(), String> {
    let dir = state.dir();
    if !bridge::is_available(&dir) {
        return Ok(());
    }
    let Ok(status) = editor_call(state, json!({"op": "status"})) else {
        return Ok(());
    };
    let Some(open) = status.get("scene_path").and_then(Value::as_str) else {
        return Ok(());
    };
    let canonical = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    if canonical(&dir.join(open)) == canonical(path) {
        return Err(format!(
            "{open} is open in the editor, which would overwrite this change when it saves. \
             Use the editor_* tools for it, or open another scene there first."
        ));
    }
    Ok(())
}

/// A scene file to change, after the editor guard.
fn scene_to_change(state: &State, args: &Value) -> Result<PathBuf, String> {
    let (_project, path) = resolve_scene(state, args)?;
    guard_open_scene(state, &path)?;
    Ok(path)
}

fn read_scene(state: &State, args: Value) -> Result<Value, String> {
    let (_project, scene_file_path) = resolve_scene(state, &args)?;
    let text = std::fs::read_to_string(&scene_file_path)
        .map_err(|e| format!("{}: {e}", scene_file_path.display()))?;
    Ok(json!({"scene": text}))
}

fn scene_summary(state: &State, args: Value) -> Result<Value, String> {
    let (_project, path) = resolve_scene(state, &args)?;
    let scene = SceneFile::load(&path)?;
    Ok(json!({
        "version": scene.version,
        "entities": scene.entities.iter().map(summary_entry).collect::<Vec<_>>(),
    }))
}

fn summary_entry(entity: &EntityData) -> Value {
    json!({
        "id": entity.id,
        "name": entity.name,
        "position": entity.position,
        "rotation": entity.rotation,
        "scale": entity.scale,
        "components": entity.components.iter().map(|c| component_label(c)).collect::<Vec<_>>(),
        "children": entity.children.iter().map(summary_entry).collect::<Vec<_>>(),
    })
}

fn create_scene(state: &State, args: Value) -> Result<Value, String> {
    let (_project, path) = resolve_scene(state, &args)?;
    if path.exists() {
        return Err(format!("already exists: {}", path.display()));
    }
    if args.get("starter").and_then(Value::as_bool).unwrap_or(false) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&path, STARTER_SCENE).map_err(|e| format!("{}: {e}", path.display()))?;
    } else {
        SceneFile::empty().save(&path)?;
    }
    Ok(json!({"created": path.display().to_string()}))
}

fn add_entity(state: &State, args: Value) -> Result<Value, String> {
    let path = scene_to_change(state, &args)?;
    let name = string_field(&args, "name")?;
    let position = vec3_field(&args, "position")?.unwrap_or([0.0, 0.0, 0.0]);
    let rotation = vec4_field(&args, "rotation")?.unwrap_or([1.0, 0.0, 0.0, 0.0]);
    let scale = vec3_field(&args, "scale")?.unwrap_or([1.0, 1.0, 1.0]);
    let parent_id = u64_field_opt(&args, "parent_id");

    let mut scene = SceneFile::load(&path)?;
    let id = fresh_id();
    scene.add(
        EntityData {
            id,
            name,
            position,
            rotation,
            scale,
            components: Vec::new(),
            children: Vec::new(),
        },
        parent_id,
    )?;
    scene.save(&path)?;
    Ok(json!({"id": id}))
}

fn remove_entity(state: &State, args: Value) -> Result<Value, String> {
    let path = scene_to_change(state, &args)?;
    let id = u64_field(&args, "id")?;
    let mut scene = SceneFile::load(&path)?;
    let removed = scene.remove(id).ok_or_else(|| format!("no entity {id}"))?;
    scene.save(&path)?;
    Ok(json!({"removed_name": removed.name}))
}

fn move_entity(state: &State, args: Value) -> Result<Value, String> {
    let path = scene_to_change(state, &args)?;
    let id = u64_field(&args, "id")?;
    let position = vec3_field(&args, "position")?;
    let rotation = vec4_field(&args, "rotation")?;
    let scale = vec3_field(&args, "scale")?;

    let mut scene = SceneFile::load(&path)?;
    let found = scene.with_entity_mut(id, |e| {
        if let Some(p) = position {
            e.position = p;
        }
        if let Some(r) = rotation {
            e.rotation = r;
        }
        if let Some(s) = scale {
            e.scale = s;
        }
    });
    if !found {
        return Err(format!("no entity {id}"));
    }
    scene.save(&path)?;
    Ok(json!({"updated": id}))
}

fn rename_entity(state: &State, args: Value) -> Result<Value, String> {
    let path = scene_to_change(state, &args)?;
    let id = u64_field(&args, "id")?;
    let name = string_field(&args, "name")?;
    let mut scene = SceneFile::load(&path)?;
    let found = scene.with_entity_mut(id, |e| e.name = name.clone());
    if !found {
        return Err(format!("no entity {id}"));
    }
    scene.save(&path)?;
    Ok(json!({"renamed": id}))
}

fn add_component(state: &State, args: Value) -> Result<Value, String> {
    let path = scene_to_change(state, &args)?;
    let id = u64_field(&args, "id")?;
    let ron_text = string_field(&args, "ron")?;
    // Make sure it is at least balanced RON; we still keep the raw text.
    ron::from_str::<ron::Value>(&ron_text).map_err(|e| format!("bad RON: {e}"))?;
    let mut scene = SceneFile::load(&path)?;
    let found = scene.with_entity_mut(id, |e| e.components.push(ron_text.clone()));
    if !found {
        return Err(format!("no entity {id}"));
    }
    scene.save(&path)?;
    Ok(json!({"added_to": id}))
}

fn remove_component(state: &State, args: Value) -> Result<Value, String> {
    let path = scene_to_change(state, &args)?;
    let id = u64_field(&args, "id")?;
    let index = usize_field(&args, "index")?;
    let mut scene = SceneFile::load(&path)?;
    let mut removed = false;
    let found = scene.with_entity_mut(id, |e| {
        if index < e.components.len() {
            e.components.remove(index);
            removed = true;
        }
    });
    if !found {
        return Err(format!("no entity {id}"));
    }
    if !removed {
        return Err(format!("component index {index} is out of range"));
    }
    scene.save(&path)?;
    Ok(json!({"removed_from": id, "index": index}))
}

// ---- live editor --------------------------------------------------------

fn editor_call(state: &State, payload: Value) -> Result<Value, String> {
    let response = bridge::call(&state.dir(), &payload)?;
    match response.get("result").and_then(|v| v.as_str()) {
        Some("ok") => Ok(response.get("data").cloned().unwrap_or(Value::Null)),
        Some("err") => Err(response
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("editor error")
            .to_string()),
        _ => Err(format!("bad bridge response: {response}")),
    }
}

/// Forward a tool's JSON `args` to the editor as `{op, ...args}`, which
/// matches the `BridgeRequest` variants one-for-one.
fn editor_pass(state: &State, args: Value, op: &str) -> Result<Value, String> {
    let mut payload = match args {
        Value::Object(map) => map,
        Value::Null => serde_json::Map::new(),
        other => return Err(format!("expected an object, got {other}")),
    };
    payload.insert("op".to_string(), Value::String(op.to_string()));
    editor_call(state, Value::Object(payload))
}

/// A JSON value as the bridge's property value. The editor converts it to
/// the property's own kind.
fn property_wire(value: &Value) -> Result<Value, String> {
    match value {
        Value::Bool(b) => Ok(json!({"kind": "Bool", "value": b})),
        Value::String(s) => Ok(json!({"kind": "Text", "value": s})),
        Value::Number(n) => match n.as_i64() {
            Some(i) => Ok(json!({"kind": "Int", "value": i})),
            None => Ok(json!({"kind": "Float", "value": n.as_f64().unwrap_or(0.0)})),
        },
        other => Err(format!("a property value is a number, true/false or text, not {other}")),
    }
}

fn editor_set_property(state: &State, args: Value) -> Result<Value, String> {
    let id = u64_field(&args, "id")?;
    let component_index = usize_field(&args, "component_index")?;
    let name = string_field(&args, "name")?;
    let value = args.get("value").ok_or_else(|| "missing value".to_string())?;
    editor_call(
        state,
        json!({
            "op": "set_property",
            "id": id,
            "component_index": component_index,
            "name": name,
            "value": property_wire(value)?,
        }),
    )
}

fn editor_open_scene(state: &State, args: Value) -> Result<Value, String> {
    let scene = string_field(&args, "scene")?;
    let discard = args.get("discard_changes").and_then(Value::as_bool).unwrap_or(false);
    editor_call(
        state,
        json!({"op": "open_scene", "path": scene, "discard_changes": discard}),
    )
}

/// Screenshots are scaled to this many pixels across unless asked
/// otherwise: enough to see a scene, a fraction of the full size.
const DEFAULT_SCREENSHOT_WIDTH: u64 = 1280;

fn viewport_screenshot(state: &State, args: Value) -> Result<Value, String> {
    let source = match args.get("source").and_then(Value::as_str).unwrap_or("viewport") {
        "viewport" | "" => 0,
        "window" => 1,
        other => return Err(format!("unknown source: {other} (viewport or window)")),
    };
    let max_width = match args.get("max_width").and_then(Value::as_u64) {
        Some(0) => Value::Null,
        Some(width) => json!(width),
        None => json!(DEFAULT_SCREENSHOT_WIDTH),
    };
    let format = args.get("format").and_then(Value::as_str).unwrap_or("png");
    editor_call(
        state,
        json!({"op": "screenshot", "source": source, "max_width": max_width, "format": format}),
    )
}

// ---- several at once ----------------------------------------------------

fn batch(state: &State, args: Value) -> Result<Value, String> {
    let operations = args
        .get("operations")
        .and_then(Value::as_array)
        .ok_or_else(|| "operations must be a list of {tool, arguments}".to_string())?;
    let stop_on_error = args.get("stop_on_error").and_then(Value::as_bool).unwrap_or(true);
    let mut results = Vec::new();
    let mut failed = 0;
    for (index, operation) in operations.iter().enumerate() {
        let tool = operation.get("tool").and_then(Value::as_str).unwrap_or_default();
        let arguments = operation.get("arguments").cloned().unwrap_or(json!({}));
        let outcome = match tool {
            "batch" => Err("a batch cannot contain a batch".to_string()),
            "viewport_screenshot" => {
                Err("take screenshots on their own, not in a batch".to_string())
            }
            "" => Err("missing tool name".to_string()),
            tool => dispatch(state, tool, arguments),
        };
        match outcome {
            Ok(result) => results.push(json!({"index": index, "tool": tool, "result": result})),
            Err(error) => {
                failed += 1;
                results.push(json!({"index": index, "tool": tool, "error": error}));
                if stop_on_error {
                    break;
                }
            }
        }
    }
    let summary = json!({
        "ran": results.len(),
        "of": operations.len(),
        "failed": failed,
        "results": results,
    });
    if failed > 0 && stop_on_error {
        Err(serde_json::to_string_pretty(&summary).unwrap_or_default())
    } else {
        Ok(summary)
    }
}

// ---- helpers ------------------------------------------------------------

fn resolve_scene(state: &State, args: &Value) -> Result<(Project, PathBuf), String> {
    let project = state.project()?;
    let scene = string_field(args, "scene")?;
    let path = scene_path(project.dir(), &scene)?;
    Ok((project, path))
}

fn string_field(args: &Value, name: &str) -> Result<String, String> {
    args.get(name)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("missing string field: {name}"))
}

fn u64_field(args: &Value, name: &str) -> Result<u64, String> {
    args.get(name)
        .and_then(|v| v.as_u64())
        .ok_or_else(|| format!("missing integer field: {name}"))
}

fn u64_field_opt(args: &Value, name: &str) -> Option<u64> {
    args.get(name).and_then(|v| v.as_u64())
}

fn usize_field(args: &Value, name: &str) -> Result<usize, String> {
    Ok(u64_field(args, name)? as usize)
}

fn vec3_field(args: &Value, name: &str) -> Result<Option<[f32; 3]>, String> {
    let Some(v) = args.get(name) else {
        return Ok(None);
    };
    let arr = v
        .as_array()
        .ok_or_else(|| format!("{name}: expected array of 3 numbers"))?;
    if arr.len() != 3 {
        return Err(format!("{name}: expected 3 numbers, got {}", arr.len()));
    }
    let mut out = [0.0; 3];
    for (i, item) in arr.iter().enumerate() {
        out[i] = item
            .as_f64()
            .ok_or_else(|| format!("{name}[{i}]: not a number"))? as f32;
    }
    Ok(Some(out))
}

fn vec4_field(args: &Value, name: &str) -> Result<Option<[f32; 4]>, String> {
    let Some(v) = args.get(name) else {
        return Ok(None);
    };
    let arr = v
        .as_array()
        .ok_or_else(|| format!("{name}: expected array of 4 numbers"))?;
    if arr.len() != 4 {
        return Err(format!("{name}: expected 4 numbers, got {}", arr.len()));
    }
    let mut out = [0.0; 4];
    for (i, item) in arr.iter().enumerate() {
        out[i] = item
            .as_f64()
            .ok_or_else(|| format!("{name}[{i}]: not a number"))? as f32;
    }
    Ok(Some(out))
}

/// What `tools/list` answers.
pub fn list_json() -> Value {
    let tools: Vec<Value> = catalog()
        .into_iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": t.input_schema,
            })
        })
        .collect();
    json!({"tools": tools})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_project(name: &str) -> (State, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "ferrite-mcp-tools-{}-{}-{}",
            name,
            std::process::id(),
            fresh_id(),
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("scenes")).unwrap();
        fs::create_dir_all(dir.join("assets/models")).unwrap();
        fs::create_dir_all(dir.join("scripts")).unwrap();
        fs::write(
            dir.join("MyGame.fp"),
            "(version:1,name:\"MyGame\",editor:())",
        )
        .unwrap();
        (State::new(dir.clone()), dir)
    }

    #[test]
    fn project_info_returns_the_name_and_version() {
        let (state, _dir) = tmp_project("info");
        let v = dispatch(&state, "project_info", json!({})).unwrap();
        assert_eq!(v["name"], Value::from("MyGame"));
        assert_eq!(v["version"], Value::from(1));
    }

    #[test]
    fn create_scene_then_summary_sees_an_empty_scene() {
        let (state, _dir) = tmp_project("create");
        dispatch(&state, "create_scene", json!({"scene": "main.ron"})).unwrap();
        let v = dispatch(&state, "scene_summary", json!({"scene": "main.ron"})).unwrap();
        assert!(v["entities"].as_array().unwrap().is_empty());
    }

    #[test]
    fn create_scene_refuses_to_overwrite() {
        let (state, _dir) = tmp_project("overwrite");
        dispatch(&state, "create_scene", json!({"scene": "main.ron"})).unwrap();
        let err = dispatch(&state, "create_scene", json!({"scene": "main.ron"}));
        assert!(err.is_err());
    }

    #[test]
    fn add_entity_adds_at_root_and_returns_an_id() {
        let (state, _dir) = tmp_project("add");
        dispatch(&state, "create_scene", json!({"scene": "main.ron"})).unwrap();
        let v = dispatch(
            &state,
            "add_entity",
            json!({"scene": "main.ron", "name": "Rock", "position": [1.0, 2.0, 3.0]}),
        )
        .unwrap();
        let id = v["id"].as_u64().unwrap();

        let summary = dispatch(&state, "scene_summary", json!({"scene": "main.ron"})).unwrap();
        let entities = summary["entities"].as_array().unwrap();
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0]["id"].as_u64().unwrap(), id);
        assert_eq!(entities[0]["name"], Value::from("Rock"));
        assert_eq!(entities[0]["position"], json!([1.0, 2.0, 3.0]));
    }

    #[test]
    fn add_entity_nests_under_a_parent() {
        let (state, _dir) = tmp_project("nest");
        dispatch(&state, "create_scene", json!({"scene": "main.ron"})).unwrap();
        let parent = dispatch(
            &state,
            "add_entity",
            json!({"scene": "main.ron", "name": "Parent"}),
        )
        .unwrap()["id"]
            .as_u64()
            .unwrap();
        dispatch(
            &state,
            "add_entity",
            json!({"scene": "main.ron", "name": "Child", "parent_id": parent}),
        )
        .unwrap();
        let summary = dispatch(&state, "scene_summary", json!({"scene": "main.ron"})).unwrap();
        let parent_entity = &summary["entities"][0];
        assert_eq!(parent_entity["children"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn move_entity_updates_the_fields_that_are_sent() {
        let (state, _dir) = tmp_project("move");
        dispatch(&state, "create_scene", json!({"scene": "main.ron"})).unwrap();
        let id = dispatch(
            &state,
            "add_entity",
            json!({"scene": "main.ron", "name": "E"}),
        )
        .unwrap()["id"]
            .as_u64()
            .unwrap();
        dispatch(
            &state,
            "move_entity",
            json!({"scene": "main.ron", "id": id, "position": [5.0, 6.0, 7.0]}),
        )
        .unwrap();
        let summary = dispatch(&state, "scene_summary", json!({"scene": "main.ron"})).unwrap();
        assert_eq!(summary["entities"][0]["position"], json!([5.0, 6.0, 7.0]));
    }

    #[test]
    fn rename_entity_changes_the_name() {
        let (state, _dir) = tmp_project("rename");
        dispatch(&state, "create_scene", json!({"scene": "main.ron"})).unwrap();
        let id = dispatch(
            &state,
            "add_entity",
            json!({"scene": "main.ron", "name": "Old"}),
        )
        .unwrap()["id"]
            .as_u64()
            .unwrap();
        dispatch(
            &state,
            "rename_entity",
            json!({"scene": "main.ron", "id": id, "name": "New"}),
        )
        .unwrap();
        let summary = dispatch(&state, "scene_summary", json!({"scene": "main.ron"})).unwrap();
        assert_eq!(summary["entities"][0]["name"], Value::from("New"));
    }

    #[test]
    fn remove_entity_removes_the_entity() {
        let (state, _dir) = tmp_project("remove");
        dispatch(&state, "create_scene", json!({"scene": "main.ron"})).unwrap();
        let id = dispatch(
            &state,
            "add_entity",
            json!({"scene": "main.ron", "name": "E"}),
        )
        .unwrap()["id"]
            .as_u64()
            .unwrap();
        dispatch(
            &state,
            "remove_entity",
            json!({"scene": "main.ron", "id": id}),
        )
        .unwrap();
        let summary = dispatch(&state, "scene_summary", json!({"scene": "main.ron"})).unwrap();
        assert!(summary["entities"].as_array().unwrap().is_empty());
    }

    #[test]
    fn add_and_remove_component_roundtrip() {
        let (state, _dir) = tmp_project("comp");
        dispatch(&state, "create_scene", json!({"scene": "main.ron"})).unwrap();
        let id = dispatch(
            &state,
            "add_entity",
            json!({"scene": "main.ron", "name": "E"}),
        )
        .unwrap()["id"]
            .as_u64()
            .unwrap();
        dispatch(
            &state,
            "add_component",
            json!({
                "scene": "main.ron",
                "id": id,
                "ron": "DebugController",
            }),
        )
        .unwrap();
        let summary = dispatch(&state, "scene_summary", json!({"scene": "main.ron"})).unwrap();
        let comps = summary["entities"][0]["components"].as_array().unwrap();
        assert_eq!(comps.len(), 1);
        assert_eq!(comps[0], Value::from("DebugController"));

        dispatch(
            &state,
            "remove_component",
            json!({"scene": "main.ron", "id": id, "index": 0}),
        )
        .unwrap();
        let summary = dispatch(&state, "scene_summary", json!({"scene": "main.ron"})).unwrap();
        assert!(summary["entities"][0]["components"]
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn add_component_rejects_malformed_ron() {
        let (state, _dir) = tmp_project("badron");
        dispatch(&state, "create_scene", json!({"scene": "main.ron"})).unwrap();
        let id = dispatch(
            &state,
            "add_entity",
            json!({"scene": "main.ron", "name": "E"}),
        )
        .unwrap()["id"]
            .as_u64()
            .unwrap();
        let err = dispatch(
            &state,
            "add_component",
            json!({"scene": "main.ron", "id": id, "ron": "(( this is not RON"}),
        );
        assert!(err.is_err());
    }

    #[test]
    fn list_scenes_assets_and_scripts_see_the_project() {
        let (state, dir) = tmp_project("lists");
        fs::write(dir.join("scenes/a.ron"), "()").unwrap();
        fs::write(dir.join("assets/models/x.obj"), "x").unwrap();
        fs::write(dir.join("scripts/s.rs"), "// script").unwrap();
        let scenes = dispatch(&state, "list_scenes", json!({})).unwrap();
        assert_eq!(scenes["scenes"], json!(["scenes/a.ron"]));
        let assets = dispatch(&state, "list_assets", json!({})).unwrap();
        assert_eq!(assets["assets"], json!(["assets/models/x.obj"]));
        let scripts = dispatch(&state, "list_scripts", json!({})).unwrap();
        assert_eq!(scripts["scripts"], json!(["scripts/s.rs"]));
    }

    #[test]
    fn set_startup_scene_updates_the_project_file() {
        let (state, _dir) = tmp_project("startup");
        dispatch(&state, "create_scene", json!({"scene": "main.ron"})).unwrap();
        dispatch(
            &state,
            "set_startup_scene",
            json!({"scene": "scenes/main.ron"}),
        )
        .unwrap();
        let info = dispatch(&state, "project_info", json!({})).unwrap();
        assert_eq!(info["startup_scene"], Value::from("scenes/main.ron"));
    }

    #[test]
    fn set_startup_scene_fails_when_the_scene_is_missing() {
        let (state, _dir) = tmp_project("missing-startup");
        let err = dispatch(
            &state,
            "set_startup_scene",
            json!({"scene": "scenes/no.ron"}),
        );
        assert!(err.is_err());
    }

    #[test]
    fn an_unknown_tool_returns_an_error() {
        let (state, _dir) = tmp_project("unknown");
        assert!(dispatch(&state, "does_not_exist", json!({})).is_err());
    }

    #[test]
    fn scene_path_rejects_traversal_through_dispatch() {
        let (state, _dir) = tmp_project("traversal");
        let err = dispatch(&state, "read_scene", json!({"scene": "../../../etc/passwd"}));
        assert!(err.is_err());
    }

    #[test]
    fn editor_tools_fail_cleanly_when_no_editor_is_open() {
        let (state, _dir) = tmp_project("no-editor");
        let err = dispatch(&state, "editor_status", json!({}));
        assert!(err.is_err());
        let msg = err.unwrap_err();
        assert!(msg.contains("editor is not open") || msg.contains("lockfile"));
    }

    #[test]
    fn editor_pass_forwards_fields_as_op() {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;

        let (state, dir) = tmp_project("forward");
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::fs::write(crate::bridge::lockfile_path(&dir), port.to_string()).unwrap();

        let handle = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            // Echo back what we saw as data so the test can assert on it.
            let request: Value = serde_json::from_str(&line).unwrap();
            let response = serde_json::json!({"result": "ok", "data": request});
            writeln!(s, "{response}").unwrap();
        });

        let v = dispatch(
            &state,
            "editor_move_entity",
            json!({"id": 42, "position": [1.0, 2.0, 3.0]}),
        )
        .unwrap();
        assert_eq!(v["op"], "move_entity");
        assert_eq!(v["id"], 42);
        assert_eq!(v["position"], json!([1.0, 2.0, 3.0]));
        handle.join().unwrap();
    }

    #[test]
    fn editor_set_property_wraps_the_value_in_a_property_wire() {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;

        let (state, dir) = tmp_project("set-prop");
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::fs::write(crate::bridge::lockfile_path(&dir), port.to_string()).unwrap();

        let handle = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            let response = serde_json::json!({"result": "ok", "data": request});
            writeln!(s, "{response}").unwrap();
        });

        let v = dispatch(
            &state,
            "editor_set_property",
            json!({
                "id": 7,
                "component_index": 0,
                "name": "speed",
                "kind": "float",
                "value": 2.5,
            }),
        )
        .unwrap();
        assert_eq!(v["op"], "set_property");
        assert_eq!(v["value"]["kind"], "Float");
        assert_eq!(v["value"]["value"], 2.5);
        handle.join().unwrap();
    }

    /// A fake editor answering `count` requests with `answer(request)`.
    fn fake_editor(
        dir: &Path,
        count: usize,
        answer: fn(&Value) -> Value,
    ) -> std::thread::JoinHandle<Vec<Value>> {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        fs::write(crate::bridge::lockfile_path(dir), port.to_string()).unwrap();
        std::thread::spawn(move || {
            let mut seen = Vec::new();
            for stream in listener.incoming().take(count) {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 {
                    continue;
                }
                let request: Value = serde_json::from_str(&line).unwrap();
                writeln!(stream, "{}", answer(&request)).unwrap();
                seen.push(request);
            }
            seen
        })
    }

    #[test]
    fn file_tools_refuse_the_scene_the_editor_has_open() {
        let (state, dir) = tmp_project("guard");
        dispatch(&state, "create_scene", json!({"scene": "main.ron"})).unwrap();
        dispatch(&state, "create_scene", json!({"scene": "other.ron"})).unwrap();
        // is_available probes once, then status is asked: two connections
        // per guarded call; two calls.
        let editor = fake_editor(&dir, 4, |_| {
            json!({"result": "ok", "data": {"scene_path": "scenes/main.ron"}})
        });
        let err = dispatch(&state, "add_entity", json!({"scene": "main.ron", "name": "X"}))
            .unwrap_err();
        assert!(err.contains("open in the editor"), "{err}");
        // Another scene is fine.
        dispatch(&state, "add_entity", json!({"scene": "other.ron", "name": "X"})).unwrap();
        editor.join().unwrap();
        let summary = dispatch(&state, "scene_summary", json!({"scene": "main.ron"})).unwrap();
        assert!(summary["entities"].as_array().unwrap().is_empty());
    }

    #[test]
    fn create_project_makes_a_project_with_a_starter_scene_and_opens_it() {
        let (state, dir) = tmp_project("create-project");
        let new_dir = dir.join("Lantern Keeper");
        let v = dispatch(
            &state,
            "create_project",
            json!({"path": new_dir.to_str().unwrap(), "player_assets": false}),
        )
        .unwrap();
        assert_eq!(v["startup_scene"], "scenes/main.ron");
        assert_eq!(state.dir(), new_dir);
        let info = dispatch(&state, "project_info", json!({})).unwrap();
        assert_eq!(info["name"], "Lantern Keeper");
        assert_eq!(info["startup_scene"], "scenes/main.ron");
        for folder in FOLDERS {
            assert!(new_dir.join(folder).is_dir(), "{folder}");
        }
        // The starter scene parses here too: camera, sky light, terrain.
        let summary = dispatch(&state, "scene_summary", json!({"scene": "main.ron"})).unwrap();
        let labels: Vec<String> = summary["entities"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|e| e["components"].as_array().unwrap().clone())
            .map(|c| c.as_str().unwrap().to_string())
            .collect();
        assert_eq!(labels, ["Camera", "SkyLight", "Terrain"]);
        // Not twice.
        assert!(dispatch(&state, "create_project", json!({"path": new_dir.to_str().unwrap()}))
            .is_err());
    }

    #[test]
    fn create_project_copies_the_player_assets() {
        let (state, dir) = tmp_project("player-assets");
        let new_dir = dir.join("with-player");
        let v = dispatch(
            &state,
            "create_project",
            json!({"path": new_dir.to_str().unwrap(), "open": false}),
        )
        .unwrap();
        assert!(v["player_assets"]["copied"].as_u64().unwrap() > 2, "{v}");
        assert!(new_dir.join("assets/models/Mannequin.fbx").is_file());
        assert!(new_dir.join("assets/animations/Idle.fbx").is_file());
        // `open: false` keeps the old project.
        assert_eq!(state.dir(), dir);
    }

    #[test]
    fn open_project_switches_projects() {
        let (state, dir) = tmp_project("open-a");
        let (_, other) = tmp_project("open-b");
        dispatch(&state, "open_project", json!({"path": other.to_str().unwrap()})).unwrap();
        assert_eq!(state.dir().canonicalize().unwrap(), other.canonicalize().unwrap());
        assert!(dispatch(&state, "open_project", json!({"path": dir.join("nothing").to_str().unwrap()}))
            .is_err());
    }

    #[test]
    fn scripts_are_written_read_and_named_like_modules() {
        let (state, dir) = tmp_project("scripts");
        let source = "use ferrite::prelude::*;\n";
        dispatch(&state, "write_script", json!({"name": "door_opener", "source": source}))
            .unwrap();
        assert_eq!(fs::read_to_string(dir.join("scripts/door_opener.rs")).unwrap(), source);
        let v = dispatch(&state, "read_script", json!({"name": "door_opener.rs"})).unwrap();
        assert_eq!(v["source"], source);
        // Not over an existing one unless asked.
        assert!(dispatch(&state, "write_script", json!({"name": "door_opener", "source": ""}))
            .is_err());
        dispatch(
            &state,
            "write_script",
            json!({"name": "door_opener", "source": "", "overwrite": true}),
        )
        .unwrap();
        for bad in ["DoorOpener", "../escape", "1st", "with space", ""] {
            assert!(
                dispatch(&state, "write_script", json!({"name": bad, "source": ""})).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn compiler_output_is_split_into_errors_and_script_warnings() {
        let dir = Path::new("/p/scripts");
        let stderr = "\
   Compiling ferrite_scripts v0.1.0
/p/scripts/lantern.rs:12:5: error[E0425]: cannot find value `x` in this scope
/p/scripts/lantern.rs:3:5: warning: unused import: `Rad`
/other/crate/src/lib.rs:1:1: warning: something in a dependency
error: could not compile `ferrite_scripts` (lib) due to 1 previous error";
        let (errors, warnings) = diagnostics(stderr, dir);
        assert_eq!(errors, ["scripts/lantern.rs:12:5: error[E0425]: cannot find value `x` in this scope"]);
        assert_eq!(warnings, ["scripts/lantern.rs:3:5: warning: unused import: `Rad`"]);
    }

    #[test]
    fn cargo_run_variables_are_recognised() {
        assert!(is_inherited_cargo_var("CARGO_PKG_NAME"));
        assert!(is_inherited_cargo_var("CARGO_MANIFEST_DIR"));
        assert!(!is_inherited_cargo_var("CARGO_HOME"));
        assert!(!is_inherited_cargo_var("CARGO"));
    }

    #[test]
    fn property_values_keep_their_json_kind() {
        assert_eq!(property_wire(&json!(true)).unwrap(), json!({"kind": "Bool", "value": true}));
        assert_eq!(property_wire(&json!(3)).unwrap(), json!({"kind": "Int", "value": 3}));
        assert_eq!(property_wire(&json!(0.5)).unwrap(), json!({"kind": "Float", "value": 0.5}));
        assert_eq!(property_wire(&json!("cube")).unwrap(), json!({"kind": "Text", "value": "cube"}));
        assert!(property_wire(&json!([1, 2])).is_err());
    }

    #[test]
    fn a_batch_runs_in_order_and_stops_at_the_first_error() {
        let (state, _dir) = tmp_project("batch");
        let v = dispatch(
            &state,
            "batch",
            json!({"operations": [
                {"tool": "create_scene", "arguments": {"scene": "a.ron"}},
                {"tool": "add_entity", "arguments": {"scene": "a.ron", "name": "One"}},
                {"tool": "add_entity", "arguments": {"scene": "a.ron", "name": "Two"}},
            ]}),
        )
        .unwrap();
        assert_eq!(v["ran"], 3);
        assert_eq!(v["failed"], 0);
        assert!(v["results"][1]["result"]["id"].as_u64().is_some());

        let err = dispatch(
            &state,
            "batch",
            json!({"operations": [
                {"tool": "add_entity", "arguments": {"scene": "a.ron", "name": "Three"}},
                {"tool": "remove_entity", "arguments": {"scene": "a.ron", "id": 1}},
                {"tool": "add_entity", "arguments": {"scene": "a.ron", "name": "Never"}},
            ]}),
        )
        .unwrap_err();
        assert!(err.contains("\"ran\": 2"), "{err}");
        let summary = dispatch(&state, "scene_summary", json!({"scene": "a.ron"})).unwrap();
        assert_eq!(summary["entities"].as_array().unwrap().len(), 3);

        // Going on after an error, when asked to.
        let v = dispatch(
            &state,
            "batch",
            json!({"stop_on_error": false, "operations": [
                {"tool": "nope"},
                {"tool": "batch", "arguments": {}},
                {"tool": "list_scenes"},
            ]}),
        )
        .unwrap();
        assert_eq!(v["ran"], 3);
        assert_eq!(v["failed"], 2);
    }

    #[test]
    fn screenshots_are_scaled_by_default() {
        let (state, dir) = tmp_project("shot");
        let editor = fake_editor(&dir, 2, |request| json!({"result": "ok", "data": request}));
        let v = dispatch(&state, "viewport_screenshot", json!({})).unwrap();
        assert_eq!(v["max_width"], 1280);
        assert_eq!(v["format"], "png");
        let v = dispatch(&state, "viewport_screenshot", json!({"max_width": 0, "format": "jpeg"}))
            .unwrap();
        assert!(v["max_width"].is_null());
        assert_eq!(v["format"], "jpeg");
        editor.join().unwrap();
    }

    #[test]
    fn list_json_includes_every_tool() {
        let listed = list_json();
        let tools = listed["tools"].as_array().unwrap();
        assert_eq!(tools.len(), catalog().len());
        for tool in tools {
            assert!(tool["name"].as_str().is_some());
            assert!(tool["description"].as_str().is_some());
            assert!(tool["inputSchema"]["type"] == Value::from("object"));
        }
    }
}
