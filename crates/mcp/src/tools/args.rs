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
        out[i] = item
            .as_f64()
            .ok_or_else(|| format!("{name}[{i}]: not a number"))? as f32;
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
        out[i] = item
            .as_f64()
            .ok_or_else(|| format!("{name}[{i}]: not a number"))? as f32;
    }
    Ok(Some(out))
}
