//! Turn `BridgeRequest`s from the MCP bridge into changes to the editor's
//! live scene, and gather the data each response needs.
//!
//! Screenshots need the rendered frame, so they are handed back to the
//! caller and answered at the end of `on_update`. Changes the editor
//! already has a command for (adding a component, opening a scene) are
//! queued as that command, after the request has been checked, so they
//! behave exactly like the same action in the UI.

use std::{io::Cursor, path::PathBuf};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use cgmath::{Deg, Point3, Quaternion, Vector3};
use ferrite::core::{
    entity::{
        component::{camera_component::CameraComponent, Property, PropertyValue},
        Entity, EntityHandle,
    },
    mcp_bridge::{
        protocol::{ComponentInfo, PropertyInfo, PropertyWire},
        BridgeRequest, BridgeResponse, EntitySummary, PendingRequest,
    },
    scene::{scene_file::ComponentData, Scene},
    script,
};
use ferrite::ron;
use serde_json::json;

use crate::editor::{
    scatter::{Counts, ScatterEdit},
    viewport::Viewport,
    Command, EditorState, Mode,
};
use ferrite::{
    core::mcp_bridge::protocol::ScatterGraphOp,
    terrain::scatter::graph::{NodeId, NodeKind, PinRef, ScatterGraph, Wire},
};

/// What a request needs from the editor.
pub struct Context<'a> {
    pub scene: &'a mut Scene,
    pub state: &'a mut EditorState,
    pub viewport: &'a mut Viewport,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ImageFormat {
    Png,
    Jpeg,
}

impl ImageFormat {
    pub fn parse(name: Option<&str>) -> Result<Self, String> {
        match name.map(str::to_ascii_lowercase).as_deref() {
            None | Some("png") => Ok(ImageFormat::Png),
            Some("jpeg") | Some("jpg") => Ok(ImageFormat::Jpeg),
            Some(other) => Err(format!("unknown image format '{other}' (png or jpeg)")),
        }
    }

    pub fn mime(self) -> &'static str {
        match self {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
        }
    }
}

/// A screenshot to take once the frame is drawn.
pub struct Shot {
    /// 0: the viewport, 1: the whole window.
    pub source: u8,
    pub max_width: Option<u32>,
    pub format: ImageFormat,
}

/// `Done`: the response has been sent. `Screenshot`: answer it after the
/// frame is drawn.
pub enum Dispatch {
    Done,
    Screenshot(PendingRequest, Shot),
}

pub fn dispatch(request: BridgeRequest, pending: PendingRequest, cx: Context) -> Dispatch {
    // Any request keeps an agent's control alive.
    if let Some(agent) = cx.state.agent.as_mut() {
        agent.last_seen = std::time::Instant::now();
    }
    let response = match request {
        BridgeRequest::Screenshot {
            source,
            max_width,
            format,
        } => match ImageFormat::parse(format.as_deref()) {
            Ok(format) => {
                return Dispatch::Screenshot(
                    pending,
                    Shot {
                        source,
                        max_width,
                        format,
                    },
                )
            }
            Err(e) => BridgeResponse::err(e),
        },
        request => respond(request, cx),
    };
    pending.respond(response);
    Dispatch::Done
}

fn respond(request: BridgeRequest, cx: Context) -> BridgeResponse {
    let Context {
        scene,
        state,
        viewport,
    } = cx;
    let result = match request {
        BridgeRequest::Ping => Ok(json!({ "alive": true })),
        BridgeRequest::Status => Ok(status(scene, state)),
        BridgeRequest::ListEntities => Ok(list_entities(scene)),
        BridgeRequest::GetEntity { id } => get_entity(scene, id),
        BridgeRequest::AddEntity {
            name,
            position,
            rotation,
            scale,
            parent_id,
        } => add_entity(scene, state, name, position, rotation, scale, parent_id),
        BridgeRequest::RemoveEntity { id } => remove_entity(scene, state, id),
        BridgeRequest::MoveEntity {
            id,
            position,
            rotation,
            scale,
        } => move_entity(scene, state, id, position, rotation, scale),
        BridgeRequest::RenameEntity { id, name } => rename_entity(scene, state, id, name),
        BridgeRequest::Select { id } => select(scene, state, id),
        BridgeRequest::AddComponent { id, ron } => add_component(scene, state, id, &ron),
        BridgeRequest::RemoveComponent { id, index } => remove_component(scene, state, id, index),
        BridgeRequest::ComponentCatalog => Ok(component_catalog()),
        BridgeRequest::OpenScene {
            path,
            discard_changes,
        } => open_scene(state, &path, discard_changes),
        BridgeRequest::ReloadScripts => {
            state.push(Command::ReloadScripts);
            Ok(json!({ "building": true, "check": "editor_status → scripts_message" }))
        }
        BridgeRequest::Focus { id } => focus(scene, state, id),
        BridgeRequest::OpenAsset { asset } => open_asset(state, &asset),
        BridgeRequest::AssetReferences { asset } => super::asset_bridge::references(&asset),
        BridgeRequest::AssetDelete { asset, confirm } => super::asset_bridge::delete(state, &asset, confirm),
        BridgeRequest::AssetRename { asset, name } => super::asset_bridge::rename(state, &asset, &name),
        BridgeRequest::AssetCreate { kind, folder, name } => super::asset_bridge::create(state, &kind, folder.as_deref(), name.as_deref()),
        BridgeRequest::AssetBrowser { category, folder, search, asset, menu, expand } => {
            super::asset_bridge::browser(state, category, folder, search, asset, menu, expand)
        }
        BridgeRequest::FolderCreate { path } => super::asset_bridge::create_folder(state, &path),
        BridgeRequest::FolderDelete { path, confirm } => super::asset_bridge::delete_folder(state, &path, confirm),
        BridgeRequest::ScatterGraphRead { asset } => scatter_graph_read(state, &asset),
        BridgeRequest::MaterialRead { asset } => super::material::bridge::read(state, &asset),
        BridgeRequest::MaterialEdit { asset, set, ops, save } => super::material::bridge::edit(state, &asset, set, ops, save),
        BridgeRequest::MaterialCreate { name, graph } => {
            // One lock at a time: the database's guard ends with the statement.
            let made = ferrite::core::material::asset::create(&mut ferrite::core::assets::database(), name.as_deref(), graph);
            made.map(|id| {
                let name = ferrite::core::assets::database().get(id).map(|e| e.meta.name.clone()).unwrap_or_default();
                json!({"created": id.to_string(), "name": name})
            })
        }
        BridgeRequest::MaterialPreview { asset, shape, wind } => super::material::bridge::preview(state, &asset, shape, wind),
        BridgeRequest::ScatterGraphEdit { asset, ops } => scatter_graph_edit(state, &asset, ops),
        BridgeRequest::MemorySnapshot { label, limit } => {
            let label = label.unwrap_or_default();
            super::stats::memory_snapshot(scene, &mut state.stats, &label).map(|(path, diff)| {
                let limit = limit.unwrap_or(30);
                match diff {
                    Some(diff) => json!({"saved": path.display().to_string(), "compared": diff_json(&diff, limit)}),
                    None => {
                        let snapshot = ferrite::core::memory::Snapshot::load(&path).ok();
                        let mut items = snapshot.map(|s| s.items).unwrap_or_default();
                        items.sort_by_key(|i| std::cmp::Reverse(i.bytes));
                        let rows: Vec<_> = items
                            .iter()
                            .take(limit)
                            .map(|i| json!({"group": i.group, "name": i.name, "count": i.count, "bytes": i.bytes, "size": ferrite::core::memory::size(i.bytes as i64)}))
                            .collect();
                        json!({"saved": path.display().to_string(), "first": true, "largest": rows})
                    }
                }
            })
        }
        BridgeRequest::MemoryDiff { before, after, limit } => memory_diff(before, after, limit.unwrap_or(30)),
        BridgeRequest::AgentControl { active, note } => {
            let now = std::time::Instant::now();
            if active {
                let since = state.agent.as_ref().map_or(now, |a| a.since);
                state.agent = Some(super::AgentControl { note: note.filter(|n| !n.trim().is_empty()), since, last_seen: now });
            } else if state.agent.take().is_some() {
                state.set_status(tr!("toolbar.agent_gave_back"));
            }
            Ok(agent_json(state))
        }
        BridgeRequest::Graphics { preset, set, reset, preview, project, save } => ferrite::core::graphics::apply_request(
            preset.as_deref(),
            set.as_ref(),
            reset.as_deref(),
            preview.as_deref(),
            project.as_ref(),
            save,
        ),
        BridgeRequest::SetDebugViews {
            vsync,
            freeze_culling,
            ambient_occlusion,
            sun_shadow_maps,
            lamp_shadows,
            ground_maps,
        } => {
            let switches = &state.debug;
            for (wanted, switch) in [
                (vsync, &switches.vsync),
                (freeze_culling, &switches.freeze_culling),
                (ambient_occlusion, &switches.ambient_occlusion),
                (sun_shadow_maps, &switches.sun_shadow_maps),
                (lamp_shadows, &switches.lamp_shadows),
                (ground_maps, &switches.ground_maps),
            ] {
                if let Some(on) = wanted {
                    switch.write(on);
                }
            }
            switches.apply();
            Ok(switches.to_json())
        }
        BridgeRequest::SetView {
            position,
            yaw_deg,
            pitch_deg,
        } => set_view(state, viewport, position, yaw_deg, pitch_deg),
        BridgeRequest::SetProperty {
            id,
            component_index,
            name,
            value,
        } => set_property(scene, state, id, component_index, &name, value),
        BridgeRequest::Play => {
            state.push(Command::Play);
            Ok(json!({ "mode": "play" }))
        }
        BridgeRequest::Stop => {
            state.push(Command::Stop);
            Ok(json!({ "mode": "edit" }))
        }
        BridgeRequest::SaveScene => save(state),
        BridgeRequest::Screenshot { .. } => unreachable!("answered after the frame"),
    };
    match result {
        Ok(serde_json::Value::Null) => BridgeResponse::ok(),
        Ok(data) => BridgeResponse::data(data),
        Err(e) => BridgeResponse::err(e),
    }
}

type Answer = Result<serde_json::Value, String>;

fn edit_mode_only(state: &EditorState, what: &str) -> Result<(), String> {
    match state.mode {
        Mode::Edit => Ok(()),
        Mode::Play => Err(format!("stop play mode first (editor_stop) to {what}")),
    }
}

/// Changes made in play mode live only until play mode stops.
fn play_note(state: &EditorState) -> Option<&'static str> {
    (state.mode == Mode::Play).then_some("play mode: this change is undone when play mode stops")
}

fn entity_exists(scene: &Scene, id: u64) -> Result<EntityHandle, String> {
    let handle = EntityHandle::from(id);
    scene
        .get_entity(&handle)
        .map(|_| handle)
        .ok_or_else(|| format!("no entity {id} (see editor_list_entities)"))
}

/// Whether an agent is at the controls, and what for.
fn agent_json(state: &EditorState) -> serde_json::Value {
    match &state.agent {
        Some(agent) => json!({
            "active": true,
            "note": agent.note,
            "seconds": agent.since.elapsed().as_secs(),
        }),
        None => json!({ "active": false }),
    }
}

fn status(scene: &Scene, state: &EditorState) -> serde_json::Value {
    let mode = match state.mode {
        Mode::Edit => "edit",
        Mode::Play => "play",
    };
    let has_camera = !scene
        .get_entities_with_component::<CameraComponent>()
        .is_empty();
    json!({
        "mode": mode,
        "project": state.project_name,
        "scene_path": state.scene_path.as_ref().map(|p| p.display().to_string()),
        "unsaved_changes": state.dirty,
        "selected": state.selected().map(u64::from),
        "status_message": state.last_status(),
        "scripts_message": state.scripts_message,
        "scripts": script::names(),
        "problems": scene.problems(),
        "has_camera": has_camera,
        "performance": state.stats.to_json(),
        "debug_views": state.debug.to_json(),
        "agent_control": agent_json(state),
        "recent_logs": recent_logs(),
    })
}

/// The newest messages of the console, warnings and errors only (scripts'
/// `info!` can be chatty), oldest first.
fn recent_logs() -> Vec<serde_json::Value> {
    ferrite::core::logging::recent(30, ferrite::log::Level::Warn)
        .iter()
        .map(|entry| {
            json!({
                "level": entry.level.as_str(),
                "source": entry.target,
                "time_s": (entry.time.as_secs_f64() * 1000.0).round() / 1000.0,
                "message": entry.message,
            })
        })
        .collect()
}

fn list_entities(scene: &Scene) -> serde_json::Value {
    let entities: Vec<EntitySummary> = scene
        .get_entities()
        .iter()
        .filter(|e| !e.is_transient())
        .map(summary)
        .collect();
    json!({ "entities": entities })
}

fn get_entity(scene: &Scene, id: u64) -> Answer {
    let handle = entity_exists(scene, id)?;
    let entity = scene.get_entity(&handle).expect("checked");
    serde_json::to_value(summary(entity)).map_err(|e| e.to_string())
}

fn summary(entity: &Entity) -> EntitySummary {
    let position = entity.get_position();
    let rotation = entity.get_rotation();
    let scale = entity.get_scale();
    EntitySummary {
        id: entity.id.into(),
        name: entity.get_name(),
        position: [position.x, position.y, position.z],
        rotation: [rotation.s, rotation.v.x, rotation.v.y, rotation.v.z],
        scale: [scale.x, scale.y, scale.z],
        components: entity
            .components()
            .iter()
            .enumerate()
            .map(|(index, c)| ComponentInfo {
                index,
                display_name: c.display_name(),
                properties: c.properties().into_iter().map(property_to_wire).collect(),
            })
            .collect(),
        children: entity
            .get_children()
            .iter()
            .filter(|c| !c.is_transient())
            .map(summary)
            .collect(),
    }
}

fn value_to_wire(value: PropertyValue) -> PropertyWire {
    match value {
        PropertyValue::Float(v) => PropertyWire::Float(v),
        PropertyValue::Int(v) => PropertyWire::Int(v),
        PropertyValue::Bool(v) => PropertyWire::Bool(v),
        PropertyValue::Text(v) => PropertyWire::Text(v),
    }
}

fn property_to_wire(p: Property) -> PropertyInfo {
    PropertyInfo {
        name: p.name.into_owned(),
        value: value_to_wire(p.value),
        read_only: p.read_only,
        range: p.range,
        options: p.options,
        action: p.action,
    }
}

fn add_entity(
    scene: &mut Scene,
    state: &mut EditorState,
    name: String,
    position: Option<[f32; 3]>,
    rotation: Option<[f32; 4]>,
    scale: Option<[f32; 3]>,
    parent_id: Option<u64>,
) -> Answer {
    let parent = parent_id.map(|id| entity_exists(scene, id)).transpose()?;
    let mut entity = Entity::new(&name);
    if let Some(p) = position {
        entity.set_local_position(Point3::new(p[0], p[1], p[2]));
    }
    if let Some(r) = rotation {
        entity.set_local_rotation(Quaternion::new(r[0], r[1], r[2], r[3]));
    }
    if let Some(s) = scale {
        entity.set_local_scale(Vector3::new(s[0], s[1], s[2]));
    }
    let id: u64 = entity.id.into();
    scene
        .add_entity_to(parent, entity)
        .map_err(|_| "the parent entity is gone".to_string())?;
    state.dirty = true;
    Ok(json!({ "id": id, "note": play_note(state) }))
}

fn remove_entity(scene: &mut Scene, state: &mut EditorState, id: u64) -> Answer {
    let handle = entity_exists(scene, id)?;
    scene.remove_entity(&handle);
    if state.selected() == Some(handle) {
        state.select(None);
    }
    state.dirty = true;
    Ok(json!({ "removed": id, "note": play_note(state) }))
}

fn move_entity(
    scene: &mut Scene,
    state: &mut EditorState,
    id: u64,
    position: Option<[f32; 3]>,
    rotation: Option<[f32; 4]>,
    scale: Option<[f32; 3]>,
) -> Answer {
    let handle = entity_exists(scene, id)?;
    if let Some(p) = position {
        scene.set_entity_position(&handle, Point3::new(p[0], p[1], p[2]));
    }
    if let Some(r) = rotation {
        scene.set_entity_rotation(&handle, Quaternion::new(r[0], r[1], r[2], r[3]));
    }
    if let Some(s) = scale {
        scene.set_entity_scale(&handle, Vector3::new(s[0], s[1], s[2]));
    }
    state.dirty = true;
    let entity = scene.get_entity(&handle).expect("checked");
    let p = entity.get_position();
    Ok(json!({ "id": id, "position": [p.x, p.y, p.z], "note": play_note(state) }))
}

fn rename_entity(scene: &mut Scene, state: &mut EditorState, id: u64, name: String) -> Answer {
    let handle = entity_exists(scene, id)?;
    scene
        .get_entity_mut(&handle)
        .expect("checked")
        .set_name(name.clone());
    state.dirty = true;
    Ok(json!({ "id": id, "name": name }))
}

fn select(scene: &Scene, state: &EditorState, id: Option<u64>) -> Answer {
    let handle = id.map(|id| entity_exists(scene, id)).transpose()?;
    state.select(handle);
    Ok(json!({ "selected": id }))
}

fn add_component(scene: &Scene, state: &mut EditorState, id: u64, text: &str) -> Answer {
    edit_mode_only(state, "add components")?;
    let handle = entity_exists(scene, id)?;
    let data: ComponentData = ron::from_str(text).map_err(|e| {
        format!("not a component: {e} (editor_component_catalog lists them with their RON)")
    })?;
    match &data {
        ComponentData::Player => {
            return Err("the Player prefab is a whole entity, not a component to add".into())
        }
        ComponentData::Script { name, data: params } => {
            // Fail here, not silently in the command.
            script::create(name, params).map_err(|e| {
                format!("{e}; loaded scripts: {}", script::names().join(", "))
            })?;
        }
        _ => {}
    }
    let index = scene.get_entity(&handle).expect("checked").components().len();
    let label = data.label();
    state.push(Command::AddComponent(handle, data));
    Ok(json!({ "id": id, "component": label, "index": index }))
}

fn remove_component(scene: &Scene, state: &mut EditorState, id: u64, index: usize) -> Answer {
    edit_mode_only(state, "remove components")?;
    let handle = entity_exists(scene, id)?;
    let components = scene.get_entity(&handle).expect("checked").components();
    let Some(component) = components.get(index) else {
        return Err(format!(
            "entity {id} has {} components; there is no index {index}",
            components.len()
        ));
    };
    let name = component.display_name();
    state.push(Command::RemoveComponent(handle, index));
    Ok(json!({ "id": id, "removed": name, "index": index }))
}

fn component_catalog() -> serde_json::Value {
    let mut entries: Vec<serde_json::Value> = ComponentData::catalog()
        .into_iter()
        .map(|(name, data)| {
            json!({
                "name": name,
                "kind": "built-in",
                "ron": ron::to_string(&data).unwrap_or_default(),
            })
        })
        .collect();
    for name in script::names() {
        let entry = match script::create(&name, &script::ScriptData::default()) {
            Ok(component) => {
                let ron = component
                    .to_data()
                    .and_then(|data| ron::to_string(&data).ok())
                    .unwrap_or_default();
                let properties: Vec<PropertyInfo> =
                    component.properties().into_iter().map(property_to_wire).collect();
                json!({ "name": name, "kind": "script", "ron": ron, "properties": properties })
            }
            Err(e) => json!({ "name": name, "kind": "script", "error": e }),
        };
        entries.push(entry);
    }
    json!({
        "components": entries,
        "note": "Prefabs (Player) are whole entities. Model components need an asset id \
                 (see list_assets and the .asset.ron files).",
    })
}

/// A scene path below the project, as typed by a tool: `main.ron` and
/// `scenes/main.ron` both name `scenes/main.ron`.
/// A comparison as the MCP tools return it: the `limit` rows that changed
/// most, and the same as a table.
fn diff_json(diff: &ferrite::core::memory::Diff, limit: usize) -> serde_json::Value {
    use ferrite::core::memory::{signed, size};
    let rows: Vec<_> = diff
        .changes
        .iter()
        .take(limit)
        .map(|c| {
            json!({
                "group": c.group,
                "name": c.name,
                "before": size(c.before as i64),
                "after": size(c.after as i64),
                "change": signed(c.delta()),
                "change_bytes": c.delta(),
                "count_before": c.count_before,
                "count_after": c.count_after,
            })
        })
        .collect();
    json!({"before": diff.before, "after": diff.after, "minutes": diff.seconds / 60, "changes": rows, "table": diff.text(limit)})
}

/// Compare two saved snapshots (see `BridgeRequest::MemoryDiff`).
fn memory_diff(before: Option<String>, after: Option<String>, limit: usize) -> Result<serde_json::Value, String> {
    use ferrite::core::memory::{self, Diff, Snapshot};
    let saved = memory::saved();
    let find = |name: &str| -> Result<PathBuf, String> {
        let path = PathBuf::from(name);
        if path.is_absolute() {
            return Ok(path);
        }
        saved
            .iter()
            .find(|p| p.file_name().is_some_and(|f| f.to_string_lossy() == name) || p.file_stem().is_some_and(|f| f.to_string_lossy() == name))
            .cloned()
            .ok_or_else(|| format!("no snapshot named {name} in {}", memory::folder().map(|d| d.display().to_string()).unwrap_or_default()))
    };
    let after = match after {
        Some(name) => find(&name)?,
        None => saved.last().cloned().ok_or("no snapshots saved yet")?,
    };
    let before = match before {
        Some(name) => find(&name)?,
        None => saved
            .iter()
            .rev()
            .find(|p| **p < after)
            .cloned()
            .ok_or("no snapshot before that one")?,
    };
    let diff = Diff::between(&Snapshot::load(&before)?, &Snapshot::load(&after)?);
    let mut result = diff_json(&diff, limit);
    result["files"] = json!([before.display().to_string(), after.display().to_string()]);
    Ok(result)
}

pub fn scene_file_path(path: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(path.trim_start_matches("./"));
    if path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("give the scene relative to the project, without '..'".into());
    }
    if path.starts_with("scenes") {
        Ok(path)
    } else {
        Ok(PathBuf::from("scenes").join(path))
    }
}

fn open_scene(state: &mut EditorState, path: &str, discard_changes: bool) -> Answer {
    edit_mode_only(state, "open a scene")?;
    // The editor works in the project folder.
    let path = scene_file_path(path)?;
    if !path.is_file() {
        return Err(format!("no scene file {}", path.display()));
    }
    if state.dirty && !discard_changes {
        return Err(
            "the open scene has unsaved changes: save them (editor_save) or pass \
             discard_changes: true"
                .into(),
        );
    }
    state.push(Command::Open(path.clone()));
    Ok(json!({ "opening": path.display().to_string() }))
}

fn save(state: &mut EditorState) -> Answer {
    edit_mode_only(state, "save")?;
    if state.scene_path.is_none() {
        return Err("the scene has no file yet; save it once in the editor (Ctrl+Shift+S)".into());
    }
    state.push(Command::Save);
    Ok(json!({ "saved": state.scene_path.as_ref().map(|p| p.display().to_string()) }))
}

fn focus(scene: &Scene, state: &mut EditorState, id: u64) -> Answer {
    edit_mode_only(state, "move the editor camera")?;
    let handle = entity_exists(scene, id)?;
    state.push(Command::Focus(handle));
    Ok(json!({ "focused": id }))
}

fn set_view(
    state: &EditorState,
    viewport: &mut Viewport,
    position: Option<[f32; 3]>,
    yaw_deg: Option<f32>,
    pitch_deg: Option<f32>,
) -> Answer {
    edit_mode_only(state, "move the editor camera")?;
    viewport.set_pose(
        position.map(|p| Point3::new(p[0], p[1], p[2])),
        yaw_deg.map(Deg),
        pitch_deg.map(Deg),
    );
    let (p, yaw, pitch) = viewport.camera_pose();
    Ok(json!({ "position": [p.x, p.y, p.z], "yaw_deg": yaw.0, "pitch_deg": pitch.0 }))
}

/// The property a tool means by `name`: exactly, with `_` for spaces (as
/// in a scene file), or ignoring case.
pub fn find_property<'a>(properties: &'a [Property], name: &str) -> Option<&'a Property> {
    let spaced = name.replace('_', " ");
    properties
        .iter()
        .find(|p| p.name == name)
        .or_else(|| properties.iter().find(|p| p.name == spaced))
        .or_else(|| {
            properties
                .iter()
                .find(|p| p.name.eq_ignore_ascii_case(&spaced))
        })
}

/// `wire` as a value of the same kind as `like` (a property's current
/// value), or why it cannot be one.
pub fn coerce(wire: PropertyWire, like: &PropertyValue) -> Result<PropertyValue, String> {
    let bad = |wanted: &str, got: &PropertyWire| Err(format!("expected {wanted}, got {got:?}"));
    match like {
        PropertyValue::Float(_) => match wire {
            PropertyWire::Float(v) => Ok(PropertyValue::Float(v)),
            PropertyWire::Int(v) => Ok(PropertyValue::Float(v as f32)),
            PropertyWire::Text(ref t) => t
                .trim()
                .parse()
                .map(PropertyValue::Float)
                .or_else(|_| bad("a number", &wire)),
            ref other => bad("a number", other),
        },
        PropertyValue::Int(_) => match wire {
            PropertyWire::Int(v) => Ok(PropertyValue::Int(v)),
            PropertyWire::Float(v) if v.fract() == 0.0 => Ok(PropertyValue::Int(v as i64)),
            PropertyWire::Text(ref t) => t
                .trim()
                .parse()
                .map(PropertyValue::Int)
                .or_else(|_| bad("a whole number", &wire)),
            ref other => bad("a whole number", other),
        },
        PropertyValue::Bool(_) => match wire {
            PropertyWire::Bool(v) => Ok(PropertyValue::Bool(v)),
            PropertyWire::Int(0) => Ok(PropertyValue::Bool(false)),
            PropertyWire::Int(1) => Ok(PropertyValue::Bool(true)),
            PropertyWire::Text(ref t) if t.eq_ignore_ascii_case("true") => {
                Ok(PropertyValue::Bool(true))
            }
            PropertyWire::Text(ref t) if t.eq_ignore_ascii_case("false") => {
                Ok(PropertyValue::Bool(false))
            }
            ref other => bad("true or false", other),
        },
        PropertyValue::Text(_) => match wire {
            PropertyWire::Text(v) => Ok(PropertyValue::Text(v)),
            PropertyWire::Float(v) => Ok(PropertyValue::Text(v.to_string())),
            PropertyWire::Int(v) => Ok(PropertyValue::Text(v.to_string())),
            PropertyWire::Bool(v) => Ok(PropertyValue::Text(v.to_string())),
        },
    }
}

/// Whether two property values are the same, floats within a little.
pub fn same_value(a: &PropertyValue, b: &PropertyValue) -> bool {
    match (a, b) {
        (PropertyValue::Float(a), PropertyValue::Float(b)) => {
            (a - b).abs() <= 1e-4 * a.abs().max(b.abs()).max(1.0)
        }
        (a, b) => a == b,
    }
}

fn set_property(
    scene: &mut Scene,
    state: &mut EditorState,
    id: u64,
    component_index: usize,
    name: &str,
    value: PropertyWire,
) -> Answer {
    let handle = entity_exists(scene, id)?;
    let entity = scene.get_entity_mut(&handle).expect("checked");
    let components = entity.components_mut();
    let count = components.len();
    let component = components.get_mut(component_index).ok_or_else(|| {
        format!("entity {id} has {count} components; there is no index {component_index}")
    })?;
    let properties = component.properties();
    let property = find_property(&properties, name).ok_or_else(|| {
        let names: Vec<&str> = properties.iter().map(|p| p.name.as_ref()).collect();
        format!(
            "{} has no property '{name}'; it has: {}",
            component.display_name(),
            names.join(", ")
        )
    })?;
    if property.read_only {
        return Err(format!("'{}' is read-only", property.name));
    }
    let mut wanted = coerce(value, &property.value)?;
    if let (PropertyValue::Text(text), Some(options)) = (&wanted, &property.options) {
        let chosen = options
            .iter()
            .find(|option| option.eq_ignore_ascii_case(text))
            .ok_or_else(|| format!("'{text}' is not one of: {}", options.join(", ")))?;
        wanted = PropertyValue::Text(chosen.clone());
    }
    let name = property.name.to_string();
    let before = property.value.clone();
    component.set_property(&name, wanted.clone());
    // A button: done, and it reads `false` again.
    if property.action {
        return Ok(json!({ "name": name, "done": true }));
    }

    let after = component
        .properties()
        .into_iter()
        .find(|p| p.name == name)
        .map(|p| p.value)
        .ok_or_else(|| format!("'{name}' disappeared after setting it"))?;
    if !same_value(&after, &wanted) && same_value(&after, &before) {
        return Err(format!(
            "'{name}' stayed {before:?}: the component did not take {wanted:?}"
        ));
    }
    state.dirty = true;
    let adjusted = (!same_value(&after, &wanted)).then(|| format!("adjusted to {after:?}"));
    Ok(json!({
        "name": name,
        "value": value_to_wire(after),
        "note": adjusted.as_deref().or(play_note(state)),
    }))
}

/// Encode an RGB snapshot, scaled down to `max_width` if wider. Returns
/// the base64 data, the size it ended up at and its MIME type.
pub fn encode_screenshot(
    rgb: Vec<u8>,
    width: u32,
    height: u32,
    max_width: Option<u32>,
    format: ImageFormat,
) -> Result<(String, u32, u32), String> {
    let mut image = image::RgbImage::from_raw(width, height, rgb)
        .ok_or_else(|| "the snapshot has the wrong size".to_string())?;
    if let Some(max) = max_width.filter(|max| *max > 0 && *max < width) {
        let new_height = ((height as u64 * max as u64) / width as u64).max(1) as u32;
        image = image::imageops::resize(&image, max, new_height, image::imageops::FilterType::Triangle);
    }
    let (w, h) = image.dimensions();
    let mut bytes = Vec::new();
    {
        use image::ImageEncoder;
        let result = match format {
            ImageFormat::Png => image::codecs::png::PngEncoder::new(Cursor::new(&mut bytes))
                .write_image(image.as_raw(), w, h, image::ExtendedColorType::Rgb8),
            ImageFormat::Jpeg => {
                image::codecs::jpeg::JpegEncoder::new_with_quality(Cursor::new(&mut bytes), 85)
                    .write_image(image.as_raw(), w, h, image::ExtendedColorType::Rgb8)
            }
        };
        result.map_err(|e| format!("encoding the screenshot: {e}"))?;
    }
    Ok((STANDARD.encode(&bytes), w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_small_png_is_encoded_at_full_size() {
        let (b64, w, h) = encode_screenshot(vec![255, 0, 0], 1, 1, None, ImageFormat::Png).unwrap();
        let bytes = STANDARD.decode(&b64).unwrap();
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!((w, h), (1, 1));
    }

    #[test]
    fn a_wide_screenshot_is_scaled_down_keeping_its_aspect() {
        let (b64, w, h) =
            encode_screenshot(vec![128; 400 * 200 * 3], 400, 200, Some(100), ImageFormat::Jpeg)
                .unwrap();
        assert_eq!((w, h), (100, 50));
        let bytes = STANDARD.decode(&b64).unwrap();
        // JPEG start-of-image marker.
        assert_eq!(&bytes[..2], b"\xff\xd8");
        // Narrower than the limit: kept as it is.
        let (_, w, _) =
            encode_screenshot(vec![0; 50 * 10 * 3], 50, 10, Some(100), ImageFormat::Png).unwrap();
        assert_eq!(w, 50);
    }

    #[test]
    fn image_formats_parse() {
        assert_eq!(ImageFormat::parse(None).unwrap(), ImageFormat::Png);
        assert_eq!(ImageFormat::parse(Some("JPG")).unwrap(), ImageFormat::Jpeg);
        assert!(ImageFormat::parse(Some("gif")).is_err());
    }

    #[test]
    fn the_catalog_offers_lights_as_ron_the_editor_reads_back() {
        let catalog = component_catalog();
        let entries = catalog["components"].as_array().expect("components");
        for name in ["Point light", "Spot light", "Directional light"] {
            let entry = entries.iter().find(|e| e["name"] == name).expect(name);
            let ron = entry["ron"].as_str().unwrap();
            let data: ComponentData = ron::from_str(ron).unwrap();
            assert!(data.light_settings().is_some(), "{name}: {ron}");
        }
    }

    #[test]
    fn light_properties_are_found_by_scene_file_names() {
        let light = ferrite::core::renderer::light::light::Light::default();
        let props = ferrite::core::entity::component::Component::properties(&light);
        assert_eq!(find_property(&props, "inner_angle").unwrap().name, "inner angle");
        assert_eq!(find_property(&props, "Color_R").unwrap().name, "color r");
    }

    #[test]
    fn properties_are_found_by_scene_file_names_and_case() {
        let props = vec![
            Property::float("time of day", 0.3),
            Property::bool("paused", true),
        ];
        assert_eq!(find_property(&props, "time of day").unwrap().name, "time of day");
        assert_eq!(find_property(&props, "time_of_day").unwrap().name, "time of day");
        assert_eq!(find_property(&props, "Paused").unwrap().name, "paused");
        assert!(find_property(&props, "speed").is_none());
    }

    #[test]
    fn values_are_converted_to_the_property_kind() {
        let float = PropertyValue::Float(0.0);
        let int = PropertyValue::Int(0);
        let flag = PropertyValue::Bool(false);
        let text = PropertyValue::Text(String::new());
        assert_eq!(coerce(PropertyWire::Int(3), &float).unwrap(), PropertyValue::Float(3.0));
        assert_eq!(
            coerce(PropertyWire::Text("0.5".into()), &float).unwrap(),
            PropertyValue::Float(0.5)
        );
        assert!(coerce(PropertyWire::Bool(true), &float).is_err());
        assert_eq!(coerce(PropertyWire::Float(4.0), &int).unwrap(), PropertyValue::Int(4));
        assert!(coerce(PropertyWire::Float(4.5), &int).is_err());
        assert_eq!(coerce(PropertyWire::Int(1), &flag).unwrap(), PropertyValue::Bool(true));
        assert_eq!(
            coerce(PropertyWire::Text("FALSE".into()), &flag).unwrap(),
            PropertyValue::Bool(false)
        );
        assert!(coerce(PropertyWire::Float(0.5), &flag).is_err());
        assert_eq!(
            coerce(PropertyWire::Float(1.5), &text).unwrap(),
            PropertyValue::Text("1.5".into())
        );
    }

    #[test]
    fn floats_compare_with_a_little_slack() {
        assert!(same_value(&PropertyValue::Float(0.3), &PropertyValue::Float(0.30000001)));
        assert!(!same_value(&PropertyValue::Float(0.3), &PropertyValue::Float(0.31)));
        assert!(!same_value(&PropertyValue::Int(1), &PropertyValue::Float(1.0)));
    }

    #[test]
    fn scene_paths_are_below_scenes_and_stay_in_the_project() {
        assert_eq!(scene_file_path("main.ron").unwrap(), PathBuf::from("scenes/main.ron"));
        assert_eq!(
            scene_file_path("scenes/level/a.ron").unwrap(),
            PathBuf::from("scenes/level/a.ron")
        );
        assert!(scene_file_path("../x.ron").is_err());
        assert!(scene_file_path("/etc/passwd").is_err());
    }
}

/// Open an asset in the asset workspace: by id, name or source path below
/// the asset folder.
fn open_asset(state: &mut EditorState, asset: &str) -> Result<serde_json::Value, String> {
    let id = resolve_asset(asset)?;
    state.push(Command::EditAsset { id, variant: None });
    Ok(json!({ "opened": id.to_string() }))
}

/// An asset by id, name or source path below the asset folder.
fn resolve_asset(asset: &str) -> Result<ferrite::core::assets::AssetId, String> {
    let database = ferrite::core::assets::database();
    let by_id = asset
        .parse::<ferrite::core::assets::AssetId>()
        .ok()
        .filter(|id| database.get(*id).is_some());
    by_id
        .or_else(|| database.iter().find(|e| e.meta.name == asset).map(|e| e.meta.id))
        .or_else(|| database.find_by_source(&database.root().join(asset)))
        .ok_or_else(|| format!("no asset '{asset}' (give its id, name or path below assets/)"))
}

fn scatter_graph_json(graph: &ScatterGraph) -> serde_json::Value {
    let nodes: Vec<serde_json::Value> = graph
        .nodes
        .iter()
        .map(|n| {
            let settings: serde_json::Map<String, serde_json::Value> = n
                .kind
                .properties()
                .into_iter()
                .map(|p| {
                    let value = match p.value {
                        PropertyValue::Float(v) => json!(v),
                        PropertyValue::Int(v) => json!(v),
                        PropertyValue::Bool(v) => json!(v),
                        PropertyValue::Text(v) => json!(v),
                    };
                    (p.name.to_string(), value)
                })
                .collect();
            json!({
                "id": n.id.0,
                "kind": n.kind.label(),
                "position": n.position,
                "inputs": n.kind.inputs().iter().map(|p| p.name).collect::<Vec<_>>(),
                "outputs": n.kind.outputs(),
                "settings": settings,
            })
        })
        .collect();
    let wires: Vec<serde_json::Value> = graph
        .wires
        .iter()
        .map(|w| json!({"from": w.from.node.0, "from_pin": w.from.pin, "to": w.to.node.0, "to_pin": w.to.pin}))
        .collect();
    json!({
        "nodes": nodes,
        "wires": wires,
        "problems": graph.problems().iter().map(|p| p.to_string()).collect::<Vec<_>>(),
    })
}

/// A scatter graph asset by id, name or path: by name, scatter graphs come
/// first (a model may have the same name).
fn resolve_scatter_graph(asset: &str) -> Result<ferrite::core::assets::AssetId, String> {
    {
        let database = ferrite::core::assets::database();
        let graph = database
            .iter()
            .find(|e| e.meta.name == asset && matches!(e.meta.kind, ferrite::core::assets::AssetKind::ScatterGraph(_)));
        if let Some(entry) = graph {
            return Ok(entry.meta.id);
        }
    }
    resolve_asset(asset)
}

fn scatter_graph_read(state: &mut EditorState, asset: &str) -> Result<serde_json::Value, String> {
    let id = resolve_scatter_graph(asset)?;
    if let Some(edit) = state.scatter.as_ref().filter(|e| e.asset == id) {
        let mut out = scatter_graph_json(&edit.graph);
        out["open_in_editor"] = json!(true);
        out["unsaved"] = json!(edit.dirty());
        out["counts"] = match &edit.counts {
            Counts::Done(counts) => json!(counts.iter().map(|(k, v)| (k.0.to_string(), json!(*v))).collect::<serde_json::Map<_, _>>()),
            Counts::Working => json!("counting"),
            Counts::None => json!("no terrain"),
            Counts::Failed(e) => json!(format!("failed: {e}")),
        };
        out["counted_on"] = json!("the 32 m detail cell under the editor camera");
        return Ok(out);
    }
    let path = {
        let database = ferrite::core::assets::database();
        let entry = database.get(id).ok_or("no such asset")?;
        if !matches!(entry.meta.kind, ferrite::core::assets::AssetKind::ScatterGraph(_)) {
            return Err(format!("'{}' is not a scatter graph", entry.meta.name));
        }
        entry.source_path()
    };
    let mut out = scatter_graph_json(&ScatterGraph::load(&path)?);
    out["open_in_editor"] = json!(false);
    Ok(out)
}

fn scatter_graph_edit(state: &mut EditorState, asset: &str, ops: Vec<ScatterGraphOp>) -> Result<serde_json::Value, String> {
    let id = resolve_scatter_graph(asset)?;
    if state.scatter.as_ref().map_or(true, |e| e.asset != id) {
        if let Some(open) = state.scatter.as_ref().filter(|e| e.dirty()) {
            return Err(format!("'{}' is open with unsaved changes; save or revert it first", open.name));
        }
        state.scatter = Some(ScatterEdit::open(id)?);
    }
    let edit = state.scatter.as_mut().expect("opened");
    // All of them, or none: worked on a copy.
    let mut graph = edit.graph.clone();
    let mut added = Vec::new();
    let mut save = false;
    let mut select: Option<Vec<NodeId>> = None;
    for (index, op) in ops.into_iter().enumerate() {
        let fail = |e: String| format!("operation {index}: {e}");
        let node = |graph: &ScatterGraph, id: u32| {
            graph.node(NodeId(id)).map(|_| NodeId(id)).ok_or_else(|| fail(format!("no node {id}")))
        };
        match op {
            ScatterGraphOp::Add { kind, position } => {
                let kind = NodeKind::catalog()
                    .into_iter()
                    .find(|k| k.is_named(&kind))
                    .ok_or_else(|| {
                        let kinds: Vec<&str> = NodeKind::catalog().iter().map(|k| k.label()).collect();
                        fail(format!("no node kind '{kind}' (one of: {})", kinds.join(", ")))
                    })?;
                let position = position.unwrap_or_else(|| {
                    let right = graph.nodes.iter().map(|n| n.position[0]).fold(0.0f32, f32::max);
                    [right + 260.0, 0.0]
                });
                added.push(graph.add_node(kind, position).0);
            }
            ScatterGraphOp::Remove { node: n } => {
                let n = node(&graph, n)?;
                graph.remove_nodes(&[n]);
            }
            ScatterGraphOp::Move { node: n, position } => {
                let n = node(&graph, n)?;
                if let Some(target) = graph.node_mut(n) {
                    target.position = position;
                }
            }
            ScatterGraphOp::Connect { from, from_pin, to, to_pin } => {
                let (from, to) = (node(&graph, from)?, node(&graph, to)?);
                graph
                    .wire(PinRef { node: from, pin: from_pin }, PinRef { node: to, pin: to_pin })
                    .map_err(fail)?;
            }
            ScatterGraphOp::Disconnect { from, from_pin, to, to_pin } => {
                graph.unwire(Wire {
                    from: PinRef { node: NodeId(from), pin: from_pin },
                    to: PinRef { node: NodeId(to), pin: to_pin },
                });
            }
            ScatterGraphOp::Set { node: n, name, value } => {
                let n = node(&graph, n)?;
                let target = graph.node_mut(n).expect("checked");
                let like = target
                    .kind
                    .properties()
                    .into_iter()
                    .find(|p| p.name == name && !p.read_only)
                    .ok_or_else(|| fail(format!("{} has no setting '{name}'", target.kind.label())))?;
                let value = match (&like.value, &value) {
                    (PropertyValue::Float(_), v) if v.is_number() => PropertyValue::Float(v.as_f64().unwrap_or(0.0) as f32),
                    (PropertyValue::Int(_), v) if v.is_number() => PropertyValue::Int(v.as_f64().unwrap_or(0.0) as i64),
                    (PropertyValue::Bool(_), serde_json::Value::Bool(b)) => PropertyValue::Bool(*b),
                    (PropertyValue::Text(_), serde_json::Value::String(s)) => PropertyValue::Text(s.clone()),
                    (like, got) => return Err(fail(format!("'{name}' takes a value like {like:?}, got {got}"))),
                };
                if !target.kind.set_property(&name, &value) {
                    return Err(fail(format!("'{name}' does not take {value:?}")));
                }
            }
            ScatterGraphOp::SetModel { node: n, asset } => {
                let n = node(&graph, n)?;
                let model = resolve_asset(&asset).map_err(fail)?;
                match graph.node_mut(n).map(|t| &mut t.kind) {
                    Some(NodeKind::SpawnMesh(spawn)) => {
                        spawn.asset = Some(ferrite::core::assets::AssetRef::whole(model));
                        spawn.model.clear();
                    }
                    _ => return Err(fail("only a Spawn mesh node has a model".into())),
                }
            }
            ScatterGraphOp::Save => save = true,
            ScatterGraphOp::Select { nodes } => {
                for n in &nodes {
                    node(&graph, *n)?;
                }
                select = Some(nodes.into_iter().map(NodeId).collect());
            }
        }
    }
    if graph != edit.graph {
        edit.edit(|g| *g = graph);
    }
    if let Some(nodes) = select {
        edit.select(nodes);
    }
    if save {
        edit.save()?;
    }
    let mut out = scatter_graph_json(&edit.graph);
    out["added"] = json!(added);
    out["unsaved"] = json!(edit.dirty());
    Ok(out)
}
