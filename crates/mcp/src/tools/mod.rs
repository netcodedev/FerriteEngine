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
};

use serde_json::{json, Value};

use crate::project::Project;

mod args;
mod catalog;
mod live;
mod projects;
mod scenes;
mod scripts;
#[cfg(test)]
mod tests;

use args::*;
pub use catalog::catalog;
use live::*;
use projects::*;
use scenes::*;
use scripts::*;

/// The scene a new project starts with; the engine writes the same one.
pub const STARTER_SCENE: &str = include_str!("../../../engine/src/core/scene/starter_scene.ron");
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
        "editor_open_asset" => editor_pass(state, args, "open_asset"),
        "editor_asset_references" => editor_pass(state, args, "asset_references"),
        "editor_asset_delete" => editor_pass(state, args, "asset_delete"),
        "editor_asset_rename" => editor_pass(state, args, "asset_rename"),
        "editor_asset_create" => editor_pass(state, args, "asset_create"),
        "editor_asset_browser" => editor_pass(state, args, "asset_browser"),
        "editor_folder_create" => editor_pass(state, args, "folder_create"),
        "editor_folder_delete" => editor_pass(state, args, "folder_delete"),
        "editor_scatter_graph_read" => editor_pass(state, args, "scatter_graph_read"),
        "editor_scatter_graph_edit" => editor_pass(state, args, "scatter_graph_edit"),
        "editor_material_read" => editor_pass(state, args, "material_read"),
        "editor_material_edit" => editor_pass(state, args, "material_edit"),
        "editor_material_preview" => editor_pass(state, args, "material_preview"),
        "editor_material_create" => editor_pass(state, args, "material_create"),
        "editor_set_view" => editor_pass(state, args, "set_view"),
        "editor_set_debug_views" => editor_pass(state, args, "set_debug_views"),
        "editor_graphics" => editor_pass(state, args, "graphics"),
        "editor_agent_control" => editor_pass(state, args, "agent_control"),
        "editor_memory_snapshot" => editor_pass(state, args, "memory_snapshot"),
        "editor_memory_diff" => editor_pass(state, args, "memory_diff"),
        "viewport_screenshot" => viewport_screenshot(state, args),
        "batch" => batch(state, args),
        other => Err(format!("unknown tool: {other}")),
    }
}

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
