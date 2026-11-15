//! The MCP stdio server: a line-oriented JSON-RPC 2.0 loop.
//!
//! Every line on stdin is one message. Requests get a response on stdout
//! (also one per line); notifications get no answer. Errors inside tools
//! come back as a response with `error`, so the client sees what
//! happened instead of the connection dropping.

use std::{
    io::{BufRead, Write},
    path::PathBuf,
};

use serde_json::{json, Value};

use crate::{
    protocol::{error_code, tool_result, Request, Response, MCP_PROTOCOL_VERSION},
    tools::{catalog, dispatch, list_json, State},
};

/// Run the server on `stdin` / `stdout` for a project rooted at
/// `project_dir`. Returns when the input stream closes.
pub fn run(project_dir: PathBuf) -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    let state = State::new(project_dir);

    for line in stdin.lock().lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(reply) = handle_line(&state, line) {
            writeln!(stdout, "{reply}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

/// Handle one line: returns `Some(json)` for a request we have an answer
/// to, `None` for a notification or a malformed message we drop silently.
pub fn handle_line(state: &State, line: &str) -> Option<String> {
    let req: Request = match serde_json::from_str(line) {
        Ok(r) => r,
        Err(e) => {
            // A parse error with no id: answer with id `null`, as JSON-RPC asks.
            let resp = Response::error(Value::Null, error_code::PARSE_ERROR, e.to_string());
            return Some(serde_json::to_string(&resp).ok()?);
        }
    };

    let id = match req.id.clone() {
        Some(id) => id,
        // A notification: no response.
        None => return None,
    };

    let resp = handle_request(state, &req.method, req.params.unwrap_or(Value::Null), id);
    serde_json::to_string(&resp).ok()
}

fn handle_request(state: &State, method: &str, params: Value, id: Value) -> Response {
    match method {
        "initialize" => Response::ok(
            id,
            json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {
                    "tools": { "listChanged": false }
                },
                "serverInfo": {
                    "name": "ferrite-mcp",
                    "version": env!("CARGO_PKG_VERSION"),
                }
            }),
        ),
        "ping" => Response::ok(id, json!({})),
        "tools/list" => Response::ok(id, list_json()),
        "tools/call" => call_tool(state, params, id),
        other => Response::error(
            id,
            error_code::METHOD_NOT_FOUND,
            format!("method not found: {other}"),
        ),
    }
}

fn call_tool(state: &State, params: Value, id: Value) -> Response {
    let name = match params.get("name").and_then(|v| v.as_str()) {
        Some(n) => n.to_string(),
        None => {
            return Response::error(id, error_code::INVALID_PARAMS, "missing tool name")
        }
    };
    let args = params.get("arguments").cloned().unwrap_or(Value::Null);
    match dispatch(state, &name, args) {
        Ok(v) => {
            if let Some(image) = image_content(&v) {
                return Response::ok(id, image);
            }
            let text = serde_json::to_string_pretty(&v).unwrap_or_default();
            Response::ok(id, tool_result(text, false))
        }
        Err(e) => Response::ok(id, tool_result(e, true)),
    }
}

/// A screenshot answer as an MCP image block (plus its size as text), or
/// `None` for any other answer.
fn image_content(value: &Value) -> Option<Value> {
    let data = value.get("image_base64")?.as_str()?;
    let mime = value
        .get("mime_type")
        .and_then(Value::as_str)
        .unwrap_or("image/png");
    let size = format!(
        "{}x{} {mime}",
        value.get("width").and_then(Value::as_u64).unwrap_or(0),
        value.get("height").and_then(Value::as_u64).unwrap_or(0)
    );
    Some(json!({
        "content": [
            { "type": "image", "data": data, "mimeType": mime },
            { "type": "text", "text": size },
        ],
        "isError": false,
    }))
}

/// Every method this server answers, for docs and tests.
pub fn methods() -> &'static [&'static str] {
    &["initialize", "ping", "tools/list", "tools/call"]
}

/// Every tool this server offers, for docs and tests.
pub fn tool_names() -> Vec<&'static str> {
    catalog().iter().map(|t| t.name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn tmp_project(name: &str) -> (State, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "ferrite-mcp-server-{}-{}-{}",
            name,
            std::process::id(),
            crate::scene::fresh_id(),
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("scenes")).unwrap();
        fs::write(
            dir.join("MyGame.fp"),
            "(version:1,name:\"MyGame\",editor:())",
        )
        .unwrap();
        (State::new(dir.clone()), dir)
    }

    fn parse_response(text: &str) -> Value {
        serde_json::from_str(text).expect("response should be JSON")
    }

    #[test]
    fn initialize_returns_the_protocol_version_and_server_info() {
        let (state, _dir) = tmp_project("init");
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
        let resp = parse_response(&handle_line(&state, line).unwrap());
        assert_eq!(resp["id"], Value::from(1));
        assert_eq!(resp["result"]["protocolVersion"], MCP_PROTOCOL_VERSION);
        assert_eq!(resp["result"]["serverInfo"]["name"], "ferrite-mcp");
        assert!(resp["result"]["capabilities"]["tools"].is_object());
    }

    #[test]
    fn ping_answers_ok() {
        let (state, _dir) = tmp_project("ping");
        let resp = parse_response(
            &handle_line(&state, r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#).unwrap(),
        );
        assert_eq!(resp["id"], Value::from(2));
        assert!(resp["result"].is_object());
    }

    #[test]
    fn tools_list_includes_every_catalog_entry() {
        let (state, _dir) = tmp_project("list");
        let resp = parse_response(
            &handle_line(&state, r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#).unwrap(),
        );
        let names: Vec<&str> = resp["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        for expected in tool_names() {
            assert!(names.contains(&expected), "missing {expected}");
        }
    }

    #[test]
    fn tools_call_wraps_the_result_in_a_content_block() {
        let (state, _dir) = tmp_project("call");
        let line = r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"project_info","arguments":{}}}"#;
        let resp = parse_response(&handle_line(&state, line).unwrap());
        let content = &resp["result"]["content"];
        assert_eq!(content[0]["type"], Value::from("text"));
        let text = content[0]["text"].as_str().unwrap();
        assert!(text.contains("MyGame"));
        assert_eq!(resp["result"]["isError"], Value::Bool(false));
    }

    #[test]
    fn a_failing_tool_returns_is_error_true_not_an_rpc_error() {
        let (state, _dir) = tmp_project("tool-err");
        let line = r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"read_scene","arguments":{"scene":"missing.ron"}}}"#;
        let resp = parse_response(&handle_line(&state, line).unwrap());
        // Tool errors travel as a successful RPC with isError=true.
        assert!(resp["error"].is_null() || !resp["error"].is_object());
        assert_eq!(resp["result"]["isError"], Value::Bool(true));
    }

    #[test]
    fn an_unknown_method_returns_an_rpc_error() {
        let (state, _dir) = tmp_project("unknown");
        let line = r#"{"jsonrpc":"2.0","id":6,"method":"does_not_exist"}"#;
        let resp = parse_response(&handle_line(&state, line).unwrap());
        assert_eq!(resp["error"]["code"], Value::from(error_code::METHOD_NOT_FOUND));
    }

    #[test]
    fn a_malformed_line_returns_a_parse_error() {
        let (state, _dir) = tmp_project("malformed");
        let resp = parse_response(&handle_line(&state, "not valid json").unwrap());
        assert_eq!(resp["error"]["code"], Value::from(error_code::PARSE_ERROR));
    }

    #[test]
    fn a_notification_produces_no_response() {
        let (state, _dir) = tmp_project("notify");
        let line = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        assert!(handle_line(&state, line).is_none());
    }

    #[test]
    fn a_screenshot_answer_becomes_an_image_block() {
        let answer = json!({"image_base64": "AAAA", "mime_type": "image/jpeg", "width": 4, "height": 2});
        let content = image_content(&answer).unwrap();
        assert_eq!(content["content"][0]["type"], "image");
        assert_eq!(content["content"][0]["mimeType"], "image/jpeg");
        assert_eq!(content["content"][1]["text"], "4x2 image/jpeg");
        assert!(image_content(&json!({"name": "x"})).is_none());
    }

    #[test]
    fn methods_and_tool_names_are_stable_lists() {
        assert!(methods().contains(&"initialize"));
        assert!(methods().contains(&"tools/list"));
        assert!(tool_names().contains(&"project_info"));
    }
}
