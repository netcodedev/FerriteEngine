//! The on-the-wire shape of a bridge message.

use serde::{Deserialize, Serialize};

/// A property value as it travels across the bridge. Mirrors the
/// component `PropertyValue` enum, but JSON-friendly.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "value")]
pub enum PropertyWire {
    Float(f32),
    Int(i64),
    Bool(bool),
    Text(String),
}

/// Full info about one property for a tool to read and edit.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PropertyInfo {
    pub name: String,
    pub value: PropertyWire,
    pub read_only: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<(f32, f32)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
    /// A button: set it `true` to do what it names (it reads `false`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub action: bool,
}

/// One component on an entity.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ComponentInfo {
    /// Zero-based index in the entity's component list.
    pub index: usize,
    pub display_name: String,
    pub properties: Vec<PropertyInfo>,
}

/// Compact view of an entity for `list_entities` / `get_entity`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EntitySummary {
    pub id: u64,
    pub name: String,
    pub position: [f32; 3],
    /// Quaternion, `[w, x, y, z]`.
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
    pub components: Vec<ComponentInfo>,
    pub children: Vec<EntitySummary>,
}

/// One change to a scatter graph (see `BridgeRequest::ScatterGraphEdit`).
/// Nodes by id; pins count from 0.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
pub enum ScatterGraphOp {
    /// Add a node of a kind by its label ("Filter: Slope", or just "Slope",
    /// ...) with its default settings.
    Add {
        kind: String,
        #[serde(default)]
        position: Option<[f32; 2]>,
    },
    Remove { node: u32 },
    Move { node: u32, position: [f32; 2] },
    Connect {
        from: u32,
        #[serde(default)]
        from_pin: u8,
        to: u32,
        #[serde(default)]
        to_pin: u8,
    },
    Disconnect {
        from: u32,
        #[serde(default)]
        from_pin: u8,
        to: u32,
        #[serde(default)]
        to_pin: u8,
    },
    /// Set a node's setting (by its name as `scatter_graph_read` lists
    /// it) to a plain JSON value.
    Set { node: u32, name: String, value: serde_json::Value },
    /// Point a `Spawn mesh` node at a model asset (id, name or path).
    SetModel { node: u32, asset: String },
    /// Write the graph to its file (terrains using it plant again).
    Save,
    /// Select nodes in the editor (the preview shows the stage of one
    /// selected node); not an edit.
    Select { nodes: Vec<u32> },
}

/// What the client sends.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum BridgeRequest {
    /// Alive? Answer with `Ok { ... }`.
    Ping,
    /// A short status: what scene is open, edit vs play mode, selection.
    Status,
    /// Every entity in the scene, with its children and components.
    ListEntities,
    /// One entity by id.
    GetEntity { id: u64 },
    /// Add a new entity (optionally under a parent). Returns the new id.
    AddEntity {
        name: String,
        #[serde(default)]
        position: Option<[f32; 3]>,
        #[serde(default)]
        rotation: Option<[f32; 4]>,
        #[serde(default)]
        scale: Option<[f32; 3]>,
        #[serde(default)]
        parent_id: Option<u64>,
    },
    /// Remove an entity (and its subtree).
    RemoveEntity { id: u64 },
    /// Update transform fields that are present.
    MoveEntity {
        id: u64,
        #[serde(default)]
        position: Option<[f32; 3]>,
        #[serde(default)]
        rotation: Option<[f32; 4]>,
        #[serde(default)]
        scale: Option<[f32; 3]>,
    },
    /// Rename an entity.
    RenameEntity { id: u64, name: String },
    /// Select an entity in the editor (so the inspector shows it).
    Select {
        #[serde(default)]
        id: Option<u64>,
    },
    /// Add a component, given as the RON of a scene file's component
    /// (e.g. `Shape(kind: Cube, color: (1.0, 0.5, 0.0))`).
    AddComponent { id: u64, ron: String },
    /// Remove the component at `index` (see `GetEntity`).
    RemoveComponent { id: u64, index: usize },
    /// Every component kind that can be added, with its default RON, and
    /// every loaded script with its parameters.
    ComponentCatalog,
    /// Open a scene file (relative to the project). Refused when the open
    /// scene has unsaved changes, unless `discard_changes`.
    OpenScene {
        path: String,
        #[serde(default)]
        discard_changes: bool,
    },
    /// Build the project's scripts again and load them.
    ReloadScripts,
    /// Point the editor camera at an entity.
    Focus { id: u64 },
    /// Open an asset (by id, name or source path below the asset folder)
    /// in the asset workspace, as a double click in the browser does.
    OpenAsset { asset: String },
    /// The files of the project that refer to an asset (by id, or a
    /// script by its path).
    AssetReferences { asset: String },
    /// Move an asset's file and sidecar to the trash. Without `confirm`
    /// only says what would go and what refers to it.
    AssetDelete {
        asset: String,
        #[serde(default)]
        confirm: bool,
    },
    /// Rename an asset: display name, source file and sidecar.
    AssetRename { asset: String, name: String },
    /// Make a new asset of `kind` (material, material_graph,
    /// scatter_graph) in `folder` below the asset folder (default: the
    /// kind's own), named `name` (default: the first free "New … N").
    AssetCreate {
        kind: String,
        #[serde(default)]
        folder: Option<String>,
        #[serde(default)]
        name: Option<String>,
    },
    /// Point the asset browser at a category (`Category::key`), a folder
    /// below the asset folder, a search, an asset to select; `menu` opens
    /// the context menu of the selected card (or of the folder).
    AssetBrowser {
        #[serde(default)]
        category: Option<String>,
        #[serde(default)]
        folder: Option<String>,
        #[serde(default)]
        search: Option<String>,
        #[serde(default)]
        asset: Option<String>,
        #[serde(default)]
        menu: bool,
        /// Unfold (true) or fold the selected pack's parts.
        #[serde(default)]
        expand: Option<bool>,
    },
    /// Make a folder (`path` below the asset folder).
    FolderCreate { path: String },
    /// Move a folder below the asset folder, with all its assets, to the
    /// trash. Without `confirm` only says what would go.
    FolderDelete {
        path: String,
        #[serde(default)]
        confirm: bool,
    },
    /// A scatter graph asset's nodes, wires, settings, problems and point
    /// counts (the editor's copy if it is open there, else the file).
    ScatterGraphRead { asset: String },
    /// Change a scatter graph asset, as one undo step in the editor's
    /// copy (opened there, without switching workspace, if it is not yet).
    ScatterGraphEdit { asset: String, ops: Vec<ScatterGraphOp> },
    /// A material asset's settings and parameters, or a material graph
    /// asset's parameters, nodes and wires (the editor's copy if it is
    /// open there, else the file).
    MaterialRead { asset: String },
    /// Change a material (`set`: property name → value) or a material
    /// graph (`ops`, see `material::edit::apply_op`) as one undo step in
    /// the editor's copy (opened there, without switching workspace, if it
    /// is not yet); `save` writes it.
    MaterialEdit {
        asset: String,
        #[serde(default)]
        set: Option<serde_json::Map<String, serde_json::Value>>,
        #[serde(default)]
        ops: Option<Vec<serde_json::Value>>,
        #[serde(default)]
        save: bool,
    },
    /// Make a new material (Standard PBR) or material graph (a copy of
    /// Standard PBR) in `assets/materials/`.
    MaterialCreate {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        graph: bool,
    },
    /// Show a material or material graph in the material workspace, on
    /// `shape` (sphere, cube, plane, cylinder), with or without wind.
    MaterialPreview {
        asset: String,
        #[serde(default)]
        shape: Option<String>,
        #[serde(default)]
        wind: Option<bool>,
    },
    /// Move or turn the editor camera; what is left out stays.
    SetView {
        #[serde(default)]
        position: Option<[f32; 3]>,
        #[serde(default)]
        yaw_deg: Option<f32>,
        #[serde(default)]
        pitch_deg: Option<f32>,
    },
    /// An agent takes the controls (moves the camera, takes screenshots,
    /// plays) or gives them back: the editor shows it, so the user keeps
    /// their hands off meanwhile. `note` says what the agent is doing.
    AgentControl {
        active: bool,
        #[serde(default)]
        note: Option<String>,
    },
    /// Read and change the graphics settings (docs/graphics-settings.md):
    /// all at once, in this order: `preset`, then each of `set` (key to a
    /// level or option name, true/false, or a number), `reset` (back to
    /// the preset's value), `preview` (a level to show everything at, or
    /// "off"), `project` (the project's part, as in its file). `save`
    /// writes this machine's choice. Returns every setting.
    Graphics {
        #[serde(default)]
        preset: Option<String>,
        #[serde(default)]
        set: Option<serde_json::Map<String, serde_json::Value>>,
        #[serde(default)]
        reset: Option<Vec<String>>,
        #[serde(default)]
        preview: Option<String>,
        #[serde(default)]
        project: Option<serde_json::Value>,
        #[serde(default)]
        save: bool,
    },
    /// Switch VSync and the renderer's debug views (as the editor's
    /// Performance panel does); what is left out stays.
    SetDebugViews {
        #[serde(default)]
        vsync: Option<bool>,
        #[serde(default)]
        freeze_culling: Option<bool>,
        #[serde(default)]
        ambient_occlusion: Option<bool>,
        #[serde(default)]
        sun_shadow_maps: Option<bool>,
        #[serde(default)]
        lamp_shadows: Option<bool>,
        #[serde(default)]
        ground_maps: Option<bool>,
    },
    /// Change one property of one component on one entity. The value is
    /// converted to the property's own kind (a number for a float, say).
    SetProperty {
        id: u64,
        component_index: usize,
        name: String,
        value: PropertyWire,
    },
    /// Toggle or set play mode.
    Play,
    Stop,
    /// Save the current scene back to its file.
    SaveScene,
    /// Capture the viewport as a PNG and return it base64-encoded.
    Screenshot {
        /// 0 = the editor viewport (default); 1 = the whole window.
        #[serde(default)]
        source: u8,
        /// Scale down to at most this many pixels across (keeps the
        /// aspect). `None` keeps the full size.
        #[serde(default)]
        max_width: Option<u32>,
        /// "png" (default) or "jpeg".
        #[serde(default)]
        format: Option<String>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum BridgeResponse {
    /// A successful answer, carrying data.
    Ok {
        #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
        data: serde_json::Value,
    },
    /// An error, with a short message.
    Err { message: String },
}

impl BridgeResponse {
    pub fn ok() -> Self {
        BridgeResponse::Ok {
            data: serde_json::Value::Null,
        }
    }

    pub fn data(data: serde_json::Value) -> Self {
        BridgeResponse::Ok { data }
    }

    pub fn err(message: impl Into<String>) -> Self {
        BridgeResponse::Err {
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_parses_from_json() {
        let text = r#"{"op":"move_entity","id":42,"position":[1.0,2.0,3.0]}"#;
        let req: BridgeRequest = serde_json::from_str(text).unwrap();
        match req {
            BridgeRequest::MoveEntity { id, position, .. } => {
                assert_eq!(id, 42);
                assert_eq!(position, Some([1.0, 2.0, 3.0]));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn missing_optional_fields_parse_as_none() {
        let text = r#"{"op":"add_entity","name":"X"}"#;
        let req: BridgeRequest = serde_json::from_str(text).unwrap();
        match req {
            BridgeRequest::AddEntity {
                name,
                position,
                parent_id,
                ..
            } => {
                assert_eq!(name, "X");
                assert_eq!(position, None);
                assert_eq!(parent_id, None);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn an_ok_response_serializes_with_data() {
        let resp = BridgeResponse::data(serde_json::json!({"id": 7}));
        let text = serde_json::to_string(&resp).unwrap();
        assert!(text.contains("\"result\":\"ok\""));
        assert!(text.contains("\"id\":7"));
    }

    #[test]
    fn an_empty_ok_response_elides_data() {
        let resp = BridgeResponse::ok();
        let text = serde_json::to_string(&resp).unwrap();
        assert!(!text.contains("\"data\""));
    }

    #[test]
    fn an_err_response_carries_the_message() {
        let resp = BridgeResponse::err("no such entity");
        let text = serde_json::to_string(&resp).unwrap();
        assert!(text.contains("\"result\":\"err\""));
        assert!(text.contains("no such entity"));
    }

    #[test]
    fn the_new_requests_parse_from_json() {
        let req: BridgeRequest =
            serde_json::from_str(r#"{"op":"add_component","id":1,"ron":"DebugController"}"#)
                .unwrap();
        assert!(matches!(req, BridgeRequest::AddComponent { id: 1, .. }));
        let req: BridgeRequest =
            serde_json::from_str(r#"{"op":"open_scene","path":"scenes/main.ron"}"#).unwrap();
        assert!(matches!(
            req,
            BridgeRequest::OpenScene {
                discard_changes: false,
                ..
            }
        ));
        let req: BridgeRequest =
            serde_json::from_str(r#"{"op":"set_view","yaw_deg":90.0}"#).unwrap();
        assert!(matches!(
            req,
            BridgeRequest::SetView {
                position: None,
                yaw_deg: Some(_),
                pitch_deg: None
            }
        ));
        let req: BridgeRequest =
            serde_json::from_str(r#"{"op":"screenshot","max_width":640,"format":"jpeg"}"#)
                .unwrap();
        assert!(matches!(
            req,
            BridgeRequest::Screenshot {
                source: 0,
                max_width: Some(640),
                ..
            }
        ));
    }

    #[test]
    fn property_wire_variants_are_round_trippable() {
        for value in [
            PropertyWire::Float(1.5),
            PropertyWire::Int(-3),
            PropertyWire::Bool(true),
            PropertyWire::Text("hi".into()),
        ] {
            let text = serde_json::to_string(&value).unwrap();
            let back: PropertyWire = serde_json::from_str(&text).unwrap();
            assert_eq!(value, back);
        }
    }
}
