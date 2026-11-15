//! Live bridge between a running editor and an external MCP server.
//!
//! The editor starts a small JSON-line TCP server on an ephemeral
//! `127.0.0.1` port and writes the port into `.ferrite-mcp.lock` in the
//! project folder, so an MCP server can find it. When a client (the
//! MCP server) sends a request, the connection thread pushes it into a
//! queue; the editor's main thread drains the queue each frame, runs
//! the request against the live scene, and sends the response back.
//!
//! The protocol is defined once, as serde-json enums, so the editor
//! never has to parse raw JSON and the MCP server never has to guess
//! at field names. Adding a request is: add a variant to
//! `BridgeRequest`, add a matching variant to `BridgeResponse`, and
//! handle it in the editor's dispatch.
//!
//! Screenshots travel as base64 PNG bytes inside a response field.

pub mod protocol;
mod server;

pub use protocol::{BridgeRequest, BridgeResponse, EntitySummary};
pub use server::{drain_pending, start, stop, PendingRequest, LOCKFILE_NAME};
