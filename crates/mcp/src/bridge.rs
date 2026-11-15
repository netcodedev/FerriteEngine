//! Client for the editor's live bridge.
//!
//! When the Ferrite editor is open on this project, it writes
//! `.ferrite-mcp.lock` at the project root with the TCP port of its
//! bridge. This module opens a connection to that port and exchanges
//! JSON-line messages with the editor.
//!
//! The MCP server uses it opportunistically: a tool that has both a
//! live and an on-disk implementation prefers the live one when the
//! bridge is reachable, else falls back to disk.

use std::{
    io::{BufRead, BufReader, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    time::Duration,
};

use serde_json::Value;

/// Lockfile the editor writes to say where its bridge is listening.
pub const LOCKFILE_NAME: &str = ".ferrite-mcp.lock";

/// Return the editor bridge's port when its lockfile is readable.
pub fn read_port(project_dir: &Path) -> Option<u16> {
    let path = lockfile_path(project_dir);
    let text = std::fs::read_to_string(path).ok()?;
    text.trim().parse().ok()
}

pub fn lockfile_path(project_dir: &Path) -> PathBuf {
    project_dir.join(LOCKFILE_NAME)
}

/// Send one request over the bridge, read one response. The request
/// is the JSON object the editor expects (e.g. `{"op":"status"}`).
pub fn call(project_dir: &Path, request: &Value) -> Result<Value, String> {
    let port = read_port(project_dir)
        .ok_or_else(|| "the editor is not open on this project".to_string())?;
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|e| {
        format!(
            "no editor answers on {addr} ({e}). {} is probably left over from an editor \
             that crashed; start the editor on this project again",
            LOCKFILE_NAME
        )
    })?;
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;

    let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    let mut writer = stream;
    let line = serde_json::to_string(request).map_err(|e| e.to_string())?;
    writeln!(writer, "{line}").map_err(|e| format!("write: {e}"))?;
    writer.flush().map_err(|e| format!("flush: {e}"))?;

    let mut response = String::new();
    reader
        .read_line(&mut response)
        .map_err(|e| format!("read: {e}"))?;
    if response.is_empty() {
        return Err("bridge closed the connection".into());
    }
    serde_json::from_str(&response).map_err(|e| format!("parse response: {e}"))
}

/// True if the editor is reachable on this project right now.
pub fn is_available(project_dir: &Path) -> bool {
    read_port(project_dir)
        .and_then(|port| {
            let addr: SocketAddr = ([127, 0, 0, 1], port).into();
            TcpStream::connect_timeout(&addr, Duration::from_millis(200)).ok()
        })
        .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn tmp(name: &str) -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(1);
        let i = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "ferrite-mcp-bridge-client-{}-{}-{}",
            name,
            std::process::id(),
            i,
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn read_port_returns_none_when_no_lockfile_is_present() {
        let dir = tmp("no-lock");
        assert!(read_port(&dir).is_none());
    }

    #[test]
    fn read_port_parses_the_number_in_the_lockfile() {
        let dir = tmp("lock");
        std::fs::write(lockfile_path(&dir), "54321\n").unwrap();
        assert_eq!(read_port(&dir), Some(54321));
    }

    #[test]
    fn read_port_rejects_a_malformed_lockfile() {
        let dir = tmp("bad-lock");
        std::fs::write(lockfile_path(&dir), "not a number").unwrap();
        assert!(read_port(&dir).is_none());
    }

    #[test]
    fn is_available_is_false_when_no_lockfile_is_present() {
        let dir = tmp("avail-no");
        assert!(!is_available(&dir));
    }

    #[test]
    fn call_fails_cleanly_when_the_editor_is_not_open() {
        let dir = tmp("no-editor");
        assert!(call(&dir, &serde_json::json!({"op":"ping"})).is_err());
    }

    #[test]
    fn a_stale_lockfile_gets_a_helpful_error() {
        let dir = tmp("stale");
        // A port nobody listens on: bind, note it, let go.
        let port = std::net::TcpListener::bind(("127.0.0.1", 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        std::fs::write(lockfile_path(&dir), port.to_string()).unwrap();
        let err = call(&dir, &serde_json::json!({"op":"ping"})).unwrap_err();
        assert!(err.contains("crashed"), "{err}");
        assert!(!is_available(&dir));
    }

    #[test]
    fn call_roundtrips_against_a_tiny_fake_editor() {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let dir = tmp("fake");
        std::fs::write(lockfile_path(&dir), port.to_string()).unwrap();

        // Fake editor: read one request line, echo back a canned response.
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            writeln!(stream, r#"{{"result":"ok","data":{{"echo":"hi"}}}}"#).unwrap();
            stream.flush().unwrap();
        });

        let v = call(&dir, &serde_json::json!({"op":"ping"})).unwrap();
        assert_eq!(v["result"], "ok");
        assert_eq!(v["data"]["echo"], "hi");

        handle.join().unwrap();
    }
}
