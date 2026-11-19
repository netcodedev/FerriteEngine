//! Reading tool arguments and the scene they name.

use std::path::PathBuf;

use serde_json::Value;

use crate::{project::Project, scene::scene_path};

use super::*;

pub(super) fn resolve_scene(state: &State, args: &Value) -> Result<(Project, PathBuf), String> {
    let project = state.project()?;
    let scene = string_field(args, "scene")?;
    let path = scene_path(project.dir(), &scene)?;
    Ok((project, path))
}

pub(super) fn string_field(args: &Value, name: &str) -> Result<String, String> {
    args.get(name)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("missing string field: {name}"))
}

pub(super) fn u64_field(args: &Value, name: &str) -> Result<u64, String> {
    args.get(name)
        .and_then(|v| v.as_u64())
        .ok_or_else(|| format!("missing integer field: {name}"))
}

pub(super) fn u64_field_opt(args: &Value, name: &str) -> Option<u64> {
    args.get(name).and_then(|v| v.as_u64())
}

pub(super) fn usize_field(args: &Value, name: &str) -> Result<usize, String> {
    Ok(u64_field(args, name)? as usize)
}

/// `item` as an f32, which must be finite (1e39 would turn into inf).
fn finite_f32(item: &Value, name: &str, i: usize) -> Result<f32, String> {
    let n = item
        .as_f64()
        .ok_or_else(|| format!("{name}[{i}]: not a number"))? as f32;
    if n.is_finite() {
        Ok(n)
    } else {
        Err(format!("{name}[{i}]: out of range"))
    }
}

pub(super) fn vec3_field(args: &Value, name: &str) -> Result<Option<[f32; 3]>, String> {
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
        out[i] = finite_f32(item, name, i)?;
    }
    Ok(Some(out))
}

pub(super) fn vec4_field(args: &Value, name: &str) -> Result<Option<[f32; 4]>, String> {
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
        out[i] = finite_f32(item, name, i)?;
    }
    Ok(Some(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn vectors_beyond_f32_range_are_rejected() {
        let args = json!({"position": [0.0, 1e39, 2.0], "rotation": [1.0, 0.0, 0.0, -1e300]});
        assert!(vec3_field(&args, "position").unwrap_err().contains("position[1]"));
        assert!(vec4_field(&args, "rotation").unwrap_err().contains("rotation[3]"));
        let ok = json!({"position": [1.0, 2.5, -3.0]});
        assert_eq!(vec3_field(&ok, "position").unwrap(), Some([1.0, 2.5, -3.0]));
    }
}
