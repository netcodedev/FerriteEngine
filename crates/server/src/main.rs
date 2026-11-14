//! A dedicated game server during development: plays a project's startup
//! scene without a window or GPU, with its scripts built and loaded as a
//! library (see `ferrite::core::runtime::run_server` for the options). An
//! exported game has its own server with the scripts built in
//! (`ferrite-game`).

// Links the engine as the shared library the project's scripts link too.
#[allow(unused_imports)]
use ferrite_dylib;

use ferrite::core::runtime::{run_server, Scripts};

fn main() {
    run_server(Scripts::Library);
}
