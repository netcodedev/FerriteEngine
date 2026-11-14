//! An exported game's dedicated server (see
//! `ferrite::core::runtime::run_server` for its options).

use ferrite::core::runtime::{run_server, Scripts};

fn main() {
    run_server(Scripts::BuiltIn(game_scripts::register));
}
