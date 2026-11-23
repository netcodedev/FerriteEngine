//! The tools this server offers, with their input schemas (`tools/list`).

use serde_json::{json, Value};

use super::*;

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
             Shape(kind: Cube, color: (1.0, 0.6, 0.2)) or a light: Light(kind: Point, \
             color: (1.0, 0.7, 0.4), intensity: 8.0, range: 12.0) (kind Point, Spot or \
             Directional; spot and directional lights shine along the entity's -Y axis; \
             every field may be left out), or particles: Particles(effect: (rate: 20.0, \
             look: Smoke)) (every field may be left out; editor_component_catalog has \
             presets: fire, smoke, embers, sparks, dust, ...). editor_component_catalog \
             lists every kind with its RON.",
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
             that could not be built, whether the scene has a camera, and the newest warnings \
             and errors of the editor's console (recent_logs).",
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
             of day or moonlight, camera speed, shape colour, light colour (\"color r\") or \
             intensity, ... Names as editor_get_entity shows them \
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
            "editor_open_asset",
            "Open an asset in the editor's asset workspace (its settings and preview; a clip \
             opens in the clip editor), as a double click in the asset browser does. Give its \
             id, name or path below assets/.",
            json!({"asset": {"type": "string"}}),
            &["asset"],
        ),
        tool(
            "editor_asset_references",
            "The files of the project that refer to an asset (id, path below assets/, or a name only \
             one asset has): sidecars and asset files naming its id (model slots → materials, \
             materials → graphs and textures, scatter graphs → models, clips → models), scenes, and \
             scripts naming its path. Each with the asset it belongs to, if any.",
            json!({"asset": {"type": "string"}}),
            &["asset"],
        ),
        tool(
            "editor_asset_delete",
            "Delete an asset: its source file and .asset.ron sidecar go to the system trash and it \
             leaves the asset database; editors open on it close (refused while they hold unsaved \
             changes). Without confirm: true nothing happens: the answer only names the asset and \
             what refers to it (those files will miss it). Ask the user before confirming.",
            json!({"asset": {"type": "string"}, "confirm": {"type": "boolean"}}),
            &["asset"],
        ),
        tool(
            "editor_asset_rename",
            "Rename an asset: its display name, source file (ending kept) and sidecar. References \
             by id stay intact; scripts naming the old path do not.",
            json!({"asset": {"type": "string"}, "name": {"type": "string"}}),
            &["asset", "name"],
        ),
        tool(
            "editor_asset_create",
            "Make a new asset: kind material (Standard PBR), material_graph (a copy of Standard \
             PBR) or scatter_graph (random points into a mesh spawner), in folder (below assets/; \
             default: materials/ or scatter/), named name (default: the first free \"New … N\"). \
             The asset browser shows it. Returns its id, name and path.",
            json!({"kind": {"type": "string", "enum": ["material", "material_graph", "scatter_graph"]}, "folder": {"type": "string"}, "name": {"type": "string"}}),
            &["kind"],
        ),
        tool(
            "editor_asset_browser",
            "Point the asset browser: category (all, models, textures, materials, material_graphs, \
             animations, scatter_graphs), folder below assets/, search text (\"\" clears it), asset \
             to select (opens its folder), menu: true to open the context menu of the selected card \
             (or of the folder) for a screenshot, expand: true/false to unfold or fold the selected \
             pack's variants (an animation file's clips). Left out stays as it is.",
            json!({"category": {"type": "string"}, "folder": {"type": "string"}, "search": {"type": "string"}, "asset": {"type": "string"}, "menu": {"type": "boolean"}, "expand": {"type": "boolean"}}),
            &[],
        ),
        tool(
            "editor_folder_create",
            "Make a folder below assets/ (path like \"props/rocks\"); the asset browser opens it.",
            json!({"path": {"type": "string"}}),
            &["path"],
        ),
        tool(
            "editor_folder_delete",
            "Delete a folder below assets/ with every asset in it (to the system trash). Without \
             confirm: true nothing happens: the answer lists the assets in it and the files outside \
             that refer to them. Ask the user before confirming.",
            json!({"path": {"type": "string"}, "confirm": {"type": "boolean"}}),
            &["path"],
        ),
        tool(
            "editor_scatter_graph_read",
            "Read a scatter graph asset (id, name or path): its nodes (id, kind, position, pins, \
             settings), wires, problems; if it is open in the editor also whether it is unsaved \
             and each node's point count on the 32 m cell under the editor camera.",
            json!({"asset": {"type": "string"}}),
            &["asset"],
        ),
        tool(
            "editor_scatter_graph_edit",
            "Change a scatter graph asset in the editor (opened there if it is not, without \
             switching workspace), all operations as one undo step, or none if one fails. Each \
             op is an object with \"do\": add {kind, position?} (kind as its label, e.g. \
             \"Filter: Slope\" or just \"Slope\"), remove {node}, move {node, position}, \
             connect {from, from_pin?, to, to_pin?}, disconnect {from, from_pin?, to, to_pin?}, \
             set {node, name, value} (a setting as editor_scatter_graph_read lists it, plain JSON \
             value), set_model {node, asset}, save, select {nodes} (the preview shows the stage of \
             one selected node; not an edit). Returns the graph and the ids of added nodes.",
            json!({"asset": {"type": "string"}, "ops": {"type": "array", "items": {"type": "object"}}}),
            &["asset", "ops"],
        ),
        tool(
            "editor_material_read",
            "A material asset (settings and parameters with their sections and types) or a material \
             graph asset (parameters, nodes with their pins and settings, wires, problems, the node \
             kinds that can be added), by id, name or path. The editor's copy if it is open there.",
            json!({"asset": {"type": "string"}}),
            &["asset"],
        ),
        tool(
            "editor_material_edit",
            "Change a material or a material graph as one undo step in the editor (opened there if it \
             is not; save: true writes it). A material: set {name: value} with the names \
             editor_material_read lists: graph, alpha (opaque/cutout), cutoff, two_sided, foliage, \
             shadows, and the graph's parameters (numbers; vectors as \"x y z\"; colours as sRGB \
             \"#rrggbb\"; textures as auto, none or a texture asset's name; a number following a \
             graphics level as \"level vegetation 0 0.1 0.2 0.3\"). A graph: ops, each one of add \
             {kind, position?} (kinds as listed; \"Param: <name>\" for a parameter), remove {node}, \
             move {node, position}, connect {from, from_pin?, to, to_pin?} (pins by number or label), \
             disconnect {to, to_pin?}, set {node, name, value}, add_param {name, type, default?, \
             group?} (types float, vec2, vec3, color, bool, color_texture, data_texture), \
             remove_param {name}, set_param {name, new_name?, label?, type?, default?, group?}. Returns what \
             editor_material_read returns, and the ids of added nodes.",
            json!({
                "asset": {"type": "string"},
                "set": {"type": "object"},
                "ops": {"type": "array", "items": {"type": "object"}},
                "save": {"type": "boolean"},
            }),
            &["asset"],
        ),
        tool(
            "editor_material_create",
            "Make a new material asset (Standard PBR, defaults) or, with graph: true, a material graph \
             asset (a copy of Standard PBR to change) in assets/materials/, named name (default: the \
             first free \"New material N\"). Returns its id and name. editor_asset_create does the \
             same for any folder and also makes scatter graphs.",
            json!({"name": {"type": "string"}, "graph": {"type": "boolean"}}),
            &[],
        ),
        tool(
            "editor_material_preview",
            "Open a material or material graph in the material workspace and show it on a shape \
             (sphere, cube, plane, cylinder), with or without wind. Take viewport_screenshot with \
             source: window afterwards to see it.",
            json!({"asset": {"type": "string"}, "shape": {"type": "string"}, "wind": {"type": "boolean"}}),
            &["asset"],
        ),
        tool(
            "editor_set_view",
            "Move or turn the editor camera (edit mode); what is left out stays. Yaw 0 looks \
             along +x, 90 along +z; pitch below 0 looks down.",
            json!({"position": vec3(), "yaw_deg": {"type": "number"}, "pitch_deg": {"type": "number"}}),
            &[],
        ),
        tool(
            "editor_agent_control",
            "Tell the user you are taking the editor's controls, or giving them back. Call it with \
             active: true BEFORE anything that moves the camera, flies, plays, takes a series of \
             screenshots or otherwise works the editor (editor_set_view, editor_play, \
             viewport_screenshot runs...), with a short note of what you do (e.g. \"measuring \
             a flight over the town\"); the editor then shows that an agent is at the controls \
             and must not be disturbed. Call it with active: false as soon as you are done, so \
             the user knows they can move the camera again. Control you give no word of (no \
             request at all) for a minute ends by itself. editor_status reports it \
             (agent_control).",
            json!({"active": {"type": "boolean"}, "note": {"type": "string"}}),
            &["active"],
        ),
        tool(
            "editor_memory_snapshot",
            "Take a RAM map snapshot of the running editor (Ctrl+Shift+M does the same): resident \
             memory by kind of mapping, Rust's heap alive and what malloc keeps freed, the GPU \
             allocator by name, and the engine's holders (chunks, caches, ray tracing structures) \
             by count and bytes, plus the heap no holder explains. Saved as RON in the log folder's \
             memory/; compared with the snapshot before (the limit rows that changed most, \
             default 30), or the largest rows for the first one. Take one, let the game run, take \
             another: what grows is at the top.",
            json!({"label": {"type": "string"}, "limit": {"type": "integer"}}),
            &[],
        ),
        tool(
            "editor_memory_diff",
            "Compare two saved RAM map snapshots (file names from the memory/ folder, or paths): \
             the last two when both are left out, the one before `after` when only that is given. \
             Returns the limit rows that changed most (default 30) and a table.",
            json!({
                "before": {"type": "string"},
                "after": {"type": "string"},
                "limit": {"type": "integer"},
            }),
            &[],
        ),
        tool(
            "editor_graphics",
            "Read and change the graphics settings (docs/graphics-settings.md). Without arguments it \
             returns every setting (key, label, group, kind, value, whether changed by hand), the \
             preset, the resolved knobs (shadow map size, distances, ...), the project's part and the \
             generation. Changes, applied in this order: preset (low/medium/high/ultra; every setting \
             to its value there), set (an object of key to value: a level name for level settings such \
             as view, shadows, vegetation, effects; an option name such as \"4x\"; true/false; a number), \
             reset (keys back to the preset's value), preview (a level to show everything at without \
             changing the choice, or \"off\"), project (the project's graphics section as in its file: \
             default_preset, settings, hidden, knobs), save (write this machine's choice now; the \
             editor also saves on its own). Changes apply at once.",
            json!({
                "preset": {"type": "string"},
                "set": {"type": "object"},
                "reset": {"type": "array", "items": {"type": "string"}},
                "preview": {"type": "string"},
                "project": {"type": "object"},
                "save": {"type": "boolean"},
            }),
            &[],
        ),
        tool(
            "editor_set_debug_views",
            "Switch VSync and the renderer's debug views, as the editor's Performance panel does; \
             what is left out stays. freeze_culling keeps culling the camera's view as it is \
             (drawn as yellow lines) while the camera moves on, to see what culling leaves out; \
             ambient_occlusion shows the ambient occlusion instead of the scene; sun_shadow_maps \
             shows the sun's shadow cascades along the bottom; lamp_shadows shows the ray traced \
             lamp shadows (the share of the shadow casting lamps' light that arrives, per colour; \
             white where none reaches) instead of the scene; ground_maps shows the ground's colour \
             from above that reflections read (near, far and horizon level) along the bottom. Returns the switches; \
             editor_status reports them (debug_views) and what culling drew (performance.culling).",
            json!({
                "vsync": {"type": "boolean"},
                "freeze_culling": {"type": "boolean"},
                "ambient_occlusion": {"type": "boolean"},
                "sun_shadow_maps": {"type": "boolean"},
                "lamp_shadows": {"type": "boolean"},
                "ground_maps": {"type": "boolean"},
            }),
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
