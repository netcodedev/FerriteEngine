//! The material workspace's MCP requests (see `BridgeRequest::Material*`).

use ferrite::core::{
    assets::{self, AssetId, AssetKind},
    entity::component::PropertyValue,
    material::{edit, preview::Shape},
};
use serde_json::{json, Map, Value};

use super::{
    super::{Command, EditorState, Workspace},
    Doc, MaterialEdit,
};

fn resolve(asset: &str) -> Result<AssetId, String> {
    let database = assets::database();
    let wanted = asset.trim();
    let found = database
        .iter()
        .filter(|e| matches!(e.meta.kind, AssetKind::Material(_) | AssetKind::MaterialGraph(_)))
        .find(|e| {
            e.meta.name.eq_ignore_ascii_case(wanted)
                || e.meta.id.to_string() == wanted
                || e.source_path() == database.root().join(wanted)
        })
        .map(|e| e.meta.id);
    found.ok_or_else(|| format!("no material or material graph '{asset}' (give its id, name or path below assets/)"))
}

fn json_of(edit: &MaterialEdit) -> Value {
    let mut out = match &edit.doc {
        Doc::Material(file) => {
            let mut out = edit::material_json(file, &edit.settings_graph());
            out["kind"] = json!("material");
            out
        }
        Doc::Graph(graph) => {
            let mut out = edit::graph_json(graph);
            out["kind"] = json!("material graph");
            out
        }
    };
    out["name"] = json!(edit.name);
    out["unsaved"] = json!(edit.dirty());
    out["problems"] = json!(edit.problems());
    out
}

/// The open copy of `id`, opened (without switching workspace) if needed.
fn open<'a>(state: &'a mut EditorState, id: AssetId) -> Result<&'a mut MaterialEdit, String> {
    if state.material.as_ref().map_or(true, |e| e.asset != id) {
        if let Some(open) = state.material.as_ref().filter(|e| e.dirty()) {
            return Err(format!("'{}' is open with unsaved changes; save or revert it first", open.name));
        }
        state.material = Some(MaterialEdit::open(id)?);
    }
    Ok(state.material.as_mut().expect("opened"))
}

pub fn read(state: &mut EditorState, asset: &str) -> Result<Value, String> {
    let id = resolve(asset)?;
    if let Some(edit) = state.material.as_ref().filter(|e| e.asset == id) {
        let mut out = json_of(edit);
        out["open_in_editor"] = json!(true);
        return Ok(out);
    }
    let mut out = json_of(&MaterialEdit::open(id)?);
    out["open_in_editor"] = json!(false);
    Ok(out)
}

fn property(value: &Value) -> PropertyValue {
    match value {
        Value::Bool(b) => PropertyValue::Bool(*b),
        Value::Number(n) => PropertyValue::Float(n.as_f64().unwrap_or(0.0) as f32),
        Value::String(s) => PropertyValue::Text(s.clone()),
        other => PropertyValue::Text(other.to_string()),
    }
}

pub fn edit(
    state: &mut EditorState,
    asset: &str,
    set: Option<Map<String, Value>>,
    ops: Option<Vec<Value>>,
    save: bool,
) -> Result<Value, String> {
    let id = resolve(asset)?;
    let edit = open(state, id)?;
    let mut added = Vec::new();
    if let Some(set) = set {
        if edit.is_graph() {
            return Err("a graph is changed with ops, not set".into());
        }
        // All of them, or none.
        let before = edit.doc.clone();
        for (name, value) in &set {
            if let Err(e) = edit.set_material_property(name, &property(value)) {
                edit.doc = before;
                edit.touch_structure();
                return Err(format!("{name}: {e}"));
            }
        }
    }
    if let Some(ops) = ops {
        if !edit.is_graph() {
            return Err("ops change a material graph; a material is changed with set".into());
        }
        added = edit.apply_ops(&ops)?.into_iter().map(|n| n.0).collect();
    }
    if save {
        edit.save()?;
    }
    let mut out = json_of(edit);
    out["added"] = json!(added);
    out["saved"] = json!(save);
    Ok(out)
}

pub fn preview(state: &mut EditorState, asset: &str, shape: Option<String>, wind: Option<bool>) -> Result<Value, String> {
    let id = resolve(asset)?;
    let edit = open(state, id)?;
    if let Some(shape) = shape {
        edit.shape = Shape::from_name(&shape).ok_or_else(|| format!("no shape '{shape}' (sphere, cube, plane, cylinder)"))?;
    }
    if let Some(wind) = wind {
        edit.wind = if wind { 1.0 } else { 0.0 };
    }
    let (shape, wind) = (edit.shape, edit.wind > 0.0);
    state.push(Command::SetWorkspace(Workspace::Material));
    Ok(json!({"shown": asset, "shape": shape.name(), "wind": wind}))
}
