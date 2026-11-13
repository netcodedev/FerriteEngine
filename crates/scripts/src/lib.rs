//! The scripts of the open project; see `Cargo.toml` and `build.rs`.

#![allow(unused_imports)]
use ferrite_dylib;

include!(concat!(env!("OUT_DIR"), "/scripts.rs"));

ferrite::export_scripts!(register);

#[cfg(test)]
#[path = "scan.rs"]
mod scan;
