//! The MCP server binary.
//!
//! Usage: `ferrite-mcp [<project_dir>]`. With no argument, serves the
//! current directory. The server speaks JSON-RPC 2.0 over stdin/stdout,
//! which is what an MCP client (Claude Desktop, Cursor, etc.) expects
//! when it launches the server as a child process.

use std::path::PathBuf;

use ferrite_mcp::server;

fn main() {
    let project_dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    if let Err(e) = server::run(project_dir) {
        eprintln!("ferrite-mcp: {e}");
        std::process::exit(1);
    }
}
