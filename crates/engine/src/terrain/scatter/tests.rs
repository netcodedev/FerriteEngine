use crate::core::assets::AssetRef;

use super::*;
use crate::core::assets::AssetId;

#[test]
fn shadows_take_a_coarser_tier_but_not_the_billboard() {
    use super::instances::shadow_lod_index;
    // Four tiers: the view goes 0, 1, 2, 3 with distance.
    assert_eq!(shadow_lod_index(10.0, 4), 1);
    assert_eq!(shadow_lod_index(100.0, 4), 2);
    assert_eq!(shadow_lod_index(200.0, 4), 2);
    // Where the view already draws the billboard, so does the shadow.
    assert_eq!(shadow_lod_index(280.0, 4), 3);
    // Two tiers: the coarser one is a mesh.
    assert_eq!(shadow_lod_index(10.0, 2), 1);
    assert_eq!(shadow_lod_index(10.0, 1), 0);
}

#[test]
fn instances_are_grouped_by_the_square_they_stand_in() {
    use super::instances::group_into_cells;
    use cgmath::{Matrix4, Vector3};
    let at = |x: f32, z: f32| Matrix4::from_translation(Vector3::new(x, 2.0, z));
    let mut instances = vec![at(1.0, 1.0), at(40.0, 1.0), at(3.0, 5.0), at(41.0, 2.0), at(20.0, 1.0)];
    let cells = group_into_cells(&mut instances, 1.0, 0.0, 10.0);
    assert_eq!(cells.len(), 3);
    // Each run's instances share a square, and the runs cover them all.
    assert_eq!(cells.iter().map(|c| c.count).sum::<u32>(), 5);
    for cell in &cells {
        let run = &instances[cell.start as usize..(cell.start + cell.count) as usize];
        let square = (run[0].w.x / 16.0).floor();
        assert!(run.iter().all(|m| (m.w.x / 16.0).floor() == square));
        // The sphere holds every instance, top to bottom.
        for m in run {
            for y in [2.0, 12.0] {
                let d = cgmath::Point3::new(m.w.x, y, m.w.z) - cell.center;
                assert!(cgmath::InnerSpace::magnitude(d) <= cell.radius + 1e-3);
            }
        }
    }
}

#[test]
fn a_layer_is_a_scatter_graph_and_reads_back_as_written() {
    let layer = ScatterLayerData::graph(AssetRef::whole(AssetId::from(0xabc)));
    let text = ron::to_string(&layer).unwrap();
    assert_eq!(ron::from_str::<ScatterLayerData>(&text).unwrap(), layer);
    // Layers of old (a model and a density) are not read any more.
    assert!(ron::from_str::<ScatterLayerData>("(model: \"Trees/Trees.gltf\", density: 21.0)").is_err());
}

#[test]
fn layers_of_different_graphs_and_outputs_have_their_own_salts() {
    let a = ScatterLayer::new(ScatterLayerData::graph(AssetRef::whole(AssetId::from(7))));
    let b = ScatterLayer::new(ScatterLayerData::graph(AssetRef::whole(AssetId::from(8))));
    assert_ne!(a.salt(), b.salt());
    assert_ne!(a.output_salt(graph::NodeId(1)), a.output_salt(graph::NodeId(2)));
}

#[test]
fn the_lighting_is_set_once_per_program_and_pass() {
    use super::instances::{current_pass, lighting_needed, LitFor};
    let lit_for = |program: u32, to_view: bool| LitFor {
        program,
        pass: current_pass(),
        view_projection: [1.0; 16],
        to_view,
        linear: true,
        ssao_depth: false,
    };
    ScatterInstances::new_pass();
    assert!(lighting_needed(Some(lit_for(3, true))));
    // The other chunks and layers of the pass.
    assert!(!lighting_needed(Some(lit_for(3, true))));
    // Another way of drawing, or another program.
    assert!(lighting_needed(Some(lit_for(3, false))));
    assert!(lighting_needed(Some(lit_for(4, false))));
    // The next pass, even with the same view.
    ScatterInstances::new_pass();
    assert!(lighting_needed(Some(lit_for(4, false))));
    assert!(!lighting_needed(Some(lit_for(4, false))));
    // Set for something else (a preview): set again after it.
    assert!(lighting_needed(None));
    assert!(lighting_needed(Some(lit_for(4, false))));
}
