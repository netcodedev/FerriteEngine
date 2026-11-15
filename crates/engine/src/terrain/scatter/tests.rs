use crate::core::assets::AssetRef;

use super::*;
use crate::core::assets::AssetId;

#[test]
fn layers_from_before_assets_still_load() {
    let layer: ScatterLayerData =
        ron::from_str("(model: \"Trees/Trees.gltf\", density: 21.0)").unwrap();
    assert_eq!(layer, ScatterLayerData::file("Trees/Trees.gltf", 21.0));
}

#[test]
fn asset_layers_round_trip_without_an_empty_path() {
    let layer = ScatterLayerData::asset(AssetRef::part(AssetId::from(0xabc), "Oak"), 12.5);
    let text = ron::to_string(&layer).unwrap();
    assert!(!text.contains("model"), "{text}");
    assert_eq!(ron::from_str::<ScatterLayerData>(&text).unwrap(), layer);

    let file = ron::to_string(&ScatterLayerData::file("rock.gltf", 3.0)).unwrap();
    assert!(!file.contains("asset"), "{file}");
}

#[test]
fn an_asset_and_a_file_scatter_to_different_spots() {
    let id = AssetId::from(7);
    let whole = ScatterLayer::new(ScatterLayerData::asset(AssetRef::whole(id), 1.0));
    let part = ScatterLayer::new(ScatterLayerData::asset(AssetRef::part(id, "Oak"), 1.0));
    let file = ScatterLayer::new(ScatterLayerData::file("rock.gltf", 1.0));
    assert_ne!(whole.salt(), part.salt());
    assert_ne!(whole.salt(), file.salt());
    // Unused: neither an asset nor a path.
    assert!(ScatterLayer::new(ScatterLayerData::file("  ", 1.0)).is_unused());
    assert!(!whole.is_unused());
}
