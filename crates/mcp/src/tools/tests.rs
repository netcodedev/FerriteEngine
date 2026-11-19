use std::fs;

use crate::scene::fresh_id;

use super::*;

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
    // The engine's `assets` folder is not in the repository (CI has
    // none): then the project is made all the same and the answer says
    // why the player assets are missing.
    let engine_assets = engine_workspace().join("assets");
    if engine_assets.join("models/Mannequin.fbx").is_file() {
        assert!(v["player_assets"]["copied"].as_u64().unwrap() > 2, "{v}");
        assert!(new_dir.join("assets/models/Mannequin.fbx").is_file());
        assert!(new_dir.join("assets/animations/Idle.fbx").is_file());
    } else {
        assert!(v["player_assets"]["copied"].is_u64() || v["player_assets"]["error"].is_string(), "{v}");
    }
    assert!(new_dir.join("scenes/main.ron").is_file());
    // `open: false` keeps the old project.
    assert_eq!(state.dir(), dir);
}

#[test]
fn the_player_model_and_animations_are_copied_from_the_engine_assets() {
    let (_, dir) = tmp_project("player-assets-from");
    let engine_assets = dir.join("engine-assets");
    let project = dir.join("new");
    fs::create_dir_all(engine_assets.join("models")).unwrap();
    fs::create_dir_all(engine_assets.join("animations")).unwrap();
    fs::write(engine_assets.join("models/Mannequin.fbx"), "model").unwrap();
    fs::write(engine_assets.join("models/Mannequin.fbx.asset.ron"), "meta").unwrap();
    fs::write(engine_assets.join("models/Other.fbx"), "other").unwrap();
    fs::write(engine_assets.join("animations/Idle.fbx"), "idle").unwrap();

    assert_eq!(copy_player_assets_from(&engine_assets, &project), Ok(3));
    assert!(project.join("assets/models/Mannequin.fbx").is_file());
    assert!(project.join("assets/models/Mannequin.fbx.asset.ron").is_file());
    assert!(project.join("assets/animations/Idle.fbx").is_file());
    assert!(!project.join("assets/models/Other.fbx").exists());
    // Files already there are kept.
    assert_eq!(copy_player_assets_from(&engine_assets, &project), Ok(0));
    assert!(copy_player_assets_from(&dir.join("nowhere"), &project).is_err());
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

#[test]
fn create_project_keeps_an_existing_main_scene() {
    let (state, dir) = tmp_project("keep-scene");
    let new_dir = dir.join("Old");
    fs::create_dir_all(new_dir.join("scenes")).unwrap();
    fs::write(new_dir.join("scenes/main.ron"), "(version: 1, entities: [])").unwrap();
    let v = dispatch(
        &state,
        "create_project",
        json!({"path": new_dir.to_str().unwrap(), "player_assets": false}),
    )
    .unwrap();
    assert!(v["starter_scene"].as_str().unwrap().starts_with("kept"));
    assert_eq!(
        fs::read_to_string(new_dir.join("scenes/main.ron")).unwrap(),
        "(version: 1, entities: [])"
    );
}

#[test]
fn add_component_rejects_text_that_breaks_the_file() {
    let (state, dir) = tmp_project("ron-prefix");
    dispatch(&state, "create_scene", json!({"scene": "main.ron"})).unwrap();
    let id = dispatch(&state, "add_entity", json!({"scene": "main.ron", "name": "E"}))
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    let before = fs::read_to_string(dir.join("scenes/main.ron")).unwrap();
    let err = dispatch(
        &state,
        "add_component",
        json!({"scene": "main.ron", "id": id, "ron": "#![enable(implicit_some)]\nDebugController"}),
    );
    assert!(err.is_err());
    assert_eq!(fs::read_to_string(dir.join("scenes/main.ron")).unwrap(), before);
    // A trailing comment is fine, also with a component after it.
    for ron in ["Light(intensity: 8.0) // warm", "DebugController"] {
        dispatch(&state, "add_component", json!({"scene": "main.ron", "id": id, "ron": ron}))
            .unwrap();
    }
    let text = fs::read_to_string(dir.join("scenes/main.ron")).unwrap();
    ron::from_str::<ron::Value>(&text).unwrap();
}

#[test]
fn set_startup_scene_stores_the_scene_it_checked() {
    let (state, dir) = tmp_project("startup-nested");
    fs::create_dir_all(dir.join("scenes/scenes")).unwrap();
    fs::write(dir.join("scenes/scenes/a.ron"), "(version: 1, entities: [])").unwrap();
    dispatch(&state, "set_startup_scene", json!({"scene": "scenes/scenes/a.ron"})).unwrap();
    let info = dispatch(&state, "project_info", json!({})).unwrap();
    assert_eq!(info["startup_scene"], "scenes/scenes/a.ron");
}
