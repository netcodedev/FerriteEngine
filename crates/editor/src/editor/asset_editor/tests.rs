use std::path::PathBuf;

use ferrite::core::assets::{AssetId, AssetKind, AssetMeta, ModelSettings};
use ferrite::core::assets::{MaterialSettings, ASSET_META_VERSION};

use super::*;

fn edit() -> AssetEdit {
    AssetEdit {
        meta: AssetMeta {
            version: ASSET_META_VERSION,
            id: AssetId::from(5),
            name: "pack".into(),
            source: "pack.gltf".into(),
            kind: AssetKind::Model(ModelSettings {
                scale: 1.0,
                variants: Vec::new(),
                materials: vec![MaterialSettings {
                    name: "bark".into(),
                    ..MaterialSettings::default()
                }],
                clips: Vec::new(),
                grip: None,
            }),
        },
        dir: PathBuf::from("assets/models/test"),
        dirty: false,
        variant: 0,
        lod: None,
        structure: 0,
        preview: 0,
        clip: None,
        holding: false,
        hold_poses: Vec::new(),
        hold_pose: 0,
    }
}

#[test]
fn giving_a_material_a_texture_marks_the_asset_changed() {
    let mut edit = edit();
    let texture = AssetId::from(9);
    edit.set_texture(0, TextureSlot::Normal, Some(texture));
    let material = &edit.meta.as_model().unwrap().materials[0];
    assert_eq!(material.normal, Some(texture));
    assert_eq!(material.base_color, None);
    assert!(edit.dirty);
    assert_eq!((edit.structure, edit.preview), (1, 1));

    // The same texture again is not a change.
    edit.dirty = false;
    edit.set_texture(0, TextureSlot::Normal, Some(texture));
    assert!(!edit.dirty);
    assert_eq!((edit.structure, edit.preview), (1, 1));

    // A material that does not exist is ignored.
    edit.set_texture(7, TextureSlot::BaseColor, Some(texture));
    assert!(!edit.dirty);
}
