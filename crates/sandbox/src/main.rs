//! Plays a project during development: the `.fp` file in the working
//! directory (or the one given as the first argument), with its scripts
//! built and loaded as a library, and its startup scene.
//! `--connect ADDRESS` joins a server right away. An exported game is
//! `ferrite-game` instead, with the scripts built in.

// Links the engine as the shared library the project's scripts link too.
#[allow(unused_imports)]
use ferrite_dylib;

use ferrite::core::runtime::{run_player, Scripts};

fn main() {
    run_player(Scripts::Library);
}
