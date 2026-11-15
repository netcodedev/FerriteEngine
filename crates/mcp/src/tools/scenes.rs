use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::{
    bridge,
    scene::{component_label, fresh_id, EntityData, SceneFile},
};

use super::*;

// ---- scene files --------------------------------------------------------

/// Refuse to change the file of the scene a running editor has open: its
/// next save would write its own state over the change.
pub(super) fn guard_open_scene(state: &State, path: &Path) -> Result<(), String> {
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
pub(super) fn scene_to_change(state: &State, args: &Value) -> Result<PathBuf, String> {
    let (_project, path) = resolve_scene(state, args)?;
    guard_open_scene(state, &path)?;
    Ok(path)
}

pub(super) fn read_scene(state: &State, args: Value) -> Result<Value, String> {
    let (_project, scene_file_path) = resolve_scene(state, &args)?;
    let text = std::fs::read_to_string(&scene_file_path)
        .map_err(|e| format!("{}: {e}", scene_file_path.display()))?;
    Ok(json!({"scene": text}))
}

pub(super) fn scene_summary(state: &State, args: Value) -> Result<Value, String> {
    let (_project, path) = resolve_scene(state, &args)?;
    let scene = SceneFile::load(&path)?;
    Ok(json!({
        "version": scene.version,
        "entities": scene.entities.iter().map(summary_entry).collect::<Vec<_>>(),
    }))
}

pub(super) fn summary_entry(entity: &EntityData) -> Value {
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

pub(super) fn create_scene(state: &State, args: Value) -> Result<Value, String> {
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

pub(super) fn add_entity(state: &State, args: Value) -> Result<Value, String> {
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

pub(super) fn remove_entity(state: &State, args: Value) -> Result<Value, String> {
    let path = scene_to_change(state, &args)?;
    let id = u64_field(&args, "id")?;
    let mut scene = SceneFile::load(&path)?;
    let removed = scene.remove(id).ok_or_else(|| format!("no entity {id}"))?;
    scene.save(&path)?;
    Ok(json!({"removed_name": removed.name}))
}

pub(super) fn move_entity(state: &State, args: Value) -> Result<Value, String> {
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

pub(super) fn rename_entity(state: &State, args: Value) -> Result<Value, String> {
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

pub(super) fn add_component(state: &State, args: Value) -> Result<Value, String> {
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

pub(super) fn remove_component(state: &State, args: Value) -> Result<Value, String> {
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
