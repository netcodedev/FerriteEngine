//! MCP (Model Context Protocol) server for Ferrite projects.
//!
//! The server speaks JSON-RPC 2.0 over stdin/stdout (the MCP transport
//! that Claude Desktop, Cursor and others open), and offers tools that
//! read and change a Ferrite project on disk without a running editor or
//! GPU: open the project file, list scenes and assets, parse a scene,
//! add or remove entities, retouch transforms.
//!
//! The server never links the engine: it mirrors the small part of the
//! scene file that it needs (see `scene::SceneFile`), and treats the rest
//! as opaque RON values that round-trip unchanged. That keeps it light and
//! means an AI can drive it on a machine without OpenGL, Assimp or GLFW.
//!
//! The entry point is `server::run` on `main.rs`; every tool lives in
//! `tools.rs`, under its own small `handle_*` function.

pub mod bridge;
pub mod project;
pub mod protocol;
pub mod scene;
pub mod server;
pub mod tools;
