
use serde_json::{json, Value};

use crate::bridge;

use super::*;

// ---- live editor --------------------------------------------------------

pub(super) fn editor_call(state: &State, payload: Value) -> Result<Value, String> {
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
pub(super) fn editor_pass(state: &State, args: Value, op: &str) -> Result<Value, String> {
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
pub(super) fn property_wire(value: &Value) -> Result<Value, String> {
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

pub(super) fn editor_set_property(state: &State, args: Value) -> Result<Value, String> {
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

pub(super) fn editor_open_scene(state: &State, args: Value) -> Result<Value, String> {
    let scene = string_field(&args, "scene")?;
    let discard = args.get("discard_changes").and_then(Value::as_bool).unwrap_or(false);
    editor_call(
        state,
        json!({"op": "open_scene", "path": scene, "discard_changes": discard}),
    )
}

/// Screenshots are scaled to this many pixels across unless asked
/// otherwise: enough to see a scene, a fraction of the full size.
pub(super) const DEFAULT_SCREENSHOT_WIDTH: u64 = 1280;

pub(super) fn viewport_screenshot(state: &State, args: Value) -> Result<Value, String> {
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
