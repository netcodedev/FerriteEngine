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
