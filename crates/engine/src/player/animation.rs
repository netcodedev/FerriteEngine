use std::{cell::RefCell, error::Error, path::Path};

use crate::core::{
    assets,
    model::animation_graph::{
        graph_file::{AnimGraphFile, ClipCache},
        AnimationGraph,
    },
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
/// parses, otherwise the built-in one. Clips refer to assets wherever
/// their files are registered.
pub fn player_graph_file() -> AnimGraphFile {
    let path = Path::new(PLAYER_GRAPH_PATH);
    let mut file = None;
    if path.exists() {
        match AnimGraphFile::load(path) {
            Ok(loaded) => file = Some(loaded),
            Err(e) => log::warn!("{PLAYER_GRAPH_PATH} is invalid, using the built-in graph: {e}"),
        }
    }
    let mut file = file.unwrap_or_else(|| {
        AnimGraphFile::from_ron(BUILT_IN_PLAYER_GRAPH).expect("built-in player graph is valid")
    });
    file.use_assets(&assets::database());
    file
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
    let mut file = AnimGraphFile::load(Path::new(path))?;
    file.use_assets(&assets::database());
    Ok(file)
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

    /// The state the inputs ask for: the one a transition leads to from
    /// `from`, or `from` if none fires.
    fn next(file: &AnimGraphFile, from: &str, inputs: &HashMap<String, f32>) -> String {
        file.transitions
            .iter()
            .filter(|t| t.from == from)
            .find(|t| t.conditions.iter().all(|c| c.evaluate(inputs)))
            .map_or(from.to_string(), |t| t.to.clone())
    }

    #[test]
    fn every_state_reaches_what_the_inputs_ask_for() {
        let file = AnimGraphFile::from_ron(BUILT_IN_PLAYER_GRAPH).unwrap();
        assert_eq!(file.default_state, "idle");
        let names = ["forward", "backward", "left", "right", "sprint", "jump"];
        for bits in 0..(1u32 << names.len()) {
            let inputs: HashMap<String, f32> = names
                .iter()
                .enumerate()
                .map(|(i, n)| (n.to_string(), ((bits >> i) & 1) as f32))
                .collect();
            let fb = inputs["forward"] - inputs["backward"];
            let lr = inputs["left"] - inputs["right"];
            let side = |left: &str, right: &str, none: &'static str| -> String {
                if lr > 0.0 {
                    left.into()
                } else if lr < 0.0 {
                    right.into()
                } else {
                    none.into()
                }
            };
            let expected = if inputs["jump"] > 0.0 {
                "jump".to_string()
            } else if fb > 0.0 && inputs["sprint"] > 0.0 {
                "fast_run".to_string()
            } else if fb > 0.0 {
                side("forward_left", "forward_right", "walk")
            } else if fb < 0.0 {
                side("backward_left", "backward_right", "back")
            } else {
                side("left", "right", "idle")
            };
            for state in &file.states {
                // One transition there, and it stays.
                let reached = next(&file, &state.name, &inputs);
                assert_eq!(reached, expected, "from {} with {inputs:?}", state.name);
                assert_eq!(next(&file, &reached, &inputs), reached);
            }
        }
    }

    #[test]
    fn graph_file_round_trips_through_ron() {
        let file = AnimGraphFile::from_ron(BUILT_IN_PLAYER_GRAPH).unwrap();
        let again = AnimGraphFile::from_ron(&file.to_ron().unwrap()).unwrap();
        assert_eq!(file, again);
    }
}
