//! An exported game's player.

// No console window next to the game on Windows.
#![cfg_attr(windows, windows_subsystem = "windows")]

use ferrite::core::runtime::{run_player, Scripts};

fn main() {
    run_player(Scripts::BuiltIn(game_scripts::register));
}
