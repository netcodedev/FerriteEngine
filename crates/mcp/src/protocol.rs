//! The MCP wire format: a thin JSON-RPC 2.0 layer.
//!
//! Only what this server needs: a request has `jsonrpc:"2.0"`, a `method`
//! and an optional `id`; a notification has no `id`; a response carries
//! `result` or `error`. Everything else (parameters, result bodies) is
//! passed through as `serde_json::Value`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The JSON-RPC version every message carries.
pub const JSONRPC_VERSION: &str = "2.0";

/// The MCP protocol version this server targets. The client's
/// `initialize` call tells us theirs; we echo it back when it is one we
/// know, else ours.
pub const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

/// Every protocol version this server can speak: it only uses features
/// that exist in all of them.
pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

#[derive(Debug, Clone, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    #[serde(default)]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Response {
    pub jsonrpc: &'static str,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ResponseError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResponseError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Standard JSON-RPC error codes.
pub mod error_code {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL_ERROR: i64 = -32603;
}

impl Response {
    pub fn ok(id: Value, result: Value) -> Self {
        Response {
            jsonrpc: JSONRPC_VERSION,
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: Value, code: i64, message: impl Into<String>) -> Self {
        Response {
            jsonrpc: JSONRPC_VERSION,
            id,
            result: None,
            error: Some(ResponseError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }
}

/// An MCP `tools/call` result: one or more content blocks and an
/// `isError` flag. We send a single text block for everything.
pub fn tool_result(text: impl Into<String>, is_error: bool) -> Value {
    serde_json::json!({
        "content": [{
            "type": "text",
            "text": text.into(),
        }],
        "isError": is_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_parses_from_json() {
        let text = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
        let req: Request = serde_json::from_str(text).unwrap();
        assert_eq!(req.jsonrpc, "2.0");
        assert_eq!(req.id.unwrap(), Value::from(1));
        assert_eq!(req.method, "ping");
        assert!(req.params.is_none());
    }

    #[test]
    fn a_notification_parses_without_an_id() {
        let text = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        let req: Request = serde_json::from_str(text).unwrap();
        assert!(req.id.is_none());
    }

    #[test]
    fn an_ok_response_serializes_without_an_error() {
        let resp = Response::ok(Value::from(7), serde_json::json!({"a": 1}));
        let text = serde_json::to_string(&resp).unwrap();
        assert!(text.contains("\"result\""));
        assert!(!text.contains("\"error\""));
    }

    #[test]
    fn an_error_response_serializes_without_a_result() {
        let resp = Response::error(Value::from(7), error_code::METHOD_NOT_FOUND, "nope");
        let text = serde_json::to_string(&resp).unwrap();
        assert!(text.contains("\"error\""));
        assert!(text.contains("-32601"));
        assert!(!text.contains("\"result\""));
    }

    #[test]
    fn a_tool_result_marks_errors_with_is_error() {
        let ok = tool_result("done", false);
        assert_eq!(ok["isError"], Value::Bool(false));
        let bad = tool_result("broken", true);
        assert_eq!(bad["isError"], Value::Bool(true));
    }
}
