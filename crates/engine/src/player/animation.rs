use std::{cell::RefCell, error::Error, path::Path};

use crate::core::model::animation_graph::{
    graph_file::{AnimGraphFile, ClipCache},
    AnimationGraph,
};

/// Where the editor saves the player's graph; preferred over the built-in
/// one when present.
pub const PLAYER_GRAPH_PATH: &str = "animations/player.animgraph.ron";

/// Locomotion graph shipped with the engine (idle, walk, strafe, run,
/// jump). Converted from the former hard-coded graph.
const BUILT_IN_PLAYER_GRAPH: &str = include_str!("player.animgraph.ron");

thread_local! {
    /// Clips stay loaded across player rebuilds (stop, scene load).
    static CLIP_CACHE: RefCell<ClipCache> = RefCell::new(ClipCache::new());
}

/// The player graph description: the project's saved file if it exists and
/// parses, otherwise the built-in one.
pub fn player_graph_file() -> AnimGraphFile {
    let path = Path::new(PLAYER_GRAPH_PATH);
    if path.exists() {
        match AnimGraphFile::load(path) {
            Ok(file) => return file,
            Err(e) => log::warn!("{PLAYER_GRAPH_PATH} is invalid, using the built-in graph: {e}"),
        }
    }
    AnimGraphFile::from_ron(BUILT_IN_PLAYER_GRAPH).expect("built-in player graph is valid")
}

/// Directory scanned for graph files offered in pickers.
pub const GRAPH_DIR: &str = "animations";
pub const GRAPH_EXTENSION: &str = ".animgraph.ron";

/// Load a graph description. The player graph path falls back to the
/// built-in graph while it has not been saved yet.
pub fn load_graph_file(path: &str) -> Result<AnimGraphFile, Box<dyn Error>> {
    if path == PLAYER_GRAPH_PATH && !Path::new(path).exists() {
        return Ok(player_graph_file());
    }
    AnimGraphFile::load(Path::new(path))
}

/// Graph files in `animations/` (plus the player graph, which may only
/// exist built-in), sorted.
pub fn animation_graph_files() -> Vec<String> {
    let mut files: Vec<String> = std::fs::read_dir(GRAPH_DIR)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.ends_with(GRAPH_EXTENSION))
                .map(|n| format!("{GRAPH_DIR}/{n}"))
                .collect()
        })
        .unwrap_or_default();
    if !files.iter().any(|f| f == PLAYER_GRAPH_PATH) {
        files.push(PLAYER_GRAPH_PATH.to_owned());
    }
    files.sort();
    files
}

/// Build a runtime graph, reusing clips loaded by earlier builds.
pub fn build_animation_graph(file: &AnimGraphFile) -> Result<AnimationGraph, Box<dyn Error>> {
    CLIP_CACHE.with(|cache| AnimationGraph::from_file(file, &mut cache.borrow_mut()))
}

pub fn default_animation_graph() -> Result<AnimationGraph, Box<dyn Error>> {
    build_animation_graph(&player_graph_file())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn built_in_graph_matches_former_hard_coded_graph() {
        let file = AnimGraphFile::from_ron(BUILT_IN_PLAYER_GRAPH).unwrap();
        assert_eq!(file.states.len(), 11);
        assert_eq!(file.transitions.len(), 38);
        assert_eq!(file.default_state, "idle");

        // idle -> walk: forward - backward > 0 && left - right == 0
        let walk = file
            .transitions
            .iter()
            .find(|t| t.from == "idle" && t.to == "walk")
            .unwrap();
        let holds = |pairs: &[(&str, f32)]| {
            let inputs: HashMap<String, f32> =
                pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect();
            walk.conditions.iter().all(|c| c.evaluate(&inputs))
        };
        assert!(holds(&[("forward", 1.0)]));
        assert!(!holds(&[("forward", 1.0), ("left", 1.0)]));
        assert!(!holds(&[("forward", 1.0), ("backward", 1.0)]));
        assert_eq!(walk.conditions[0].describe(), "forward - backward > 0");
    }

    #[test]
    fn graph_file_round_trips_through_ron() {
        let file = AnimGraphFile::from_ron(BUILT_IN_PLAYER_GRAPH).unwrap();
        let again = AnimGraphFile::from_ron(&file.to_ron().unwrap()).unwrap();
        assert_eq!(file, again);
    }
}
