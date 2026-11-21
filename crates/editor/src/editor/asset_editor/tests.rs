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
                anchors: Vec::new(),
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
        others: Vec::new(),
        anchor: None,
    }
}

#[test]
fn anchors_of_the_asset_are_added_renamed_and_removed() {
    let mut edit = edit();
    let id = edit.meta.id;
    edit.add_anchor(id, None, [0.0, 0.7, 0.0]);
    edit.add_anchor(id, Some("mixamorig1:Spine2".into()), [0.0; 3]);
    let names: Vec<String> = edit.anchors(id).into_iter().map(|a| a.name).collect();
    assert_eq!(names, ["anchor", "Spine2"]);
    assert!(edit.dirty);
    // The new one is selected.
    assert_eq!(edit.anchor, Some(AnchorPick { asset: id, name: "Spine2".into() }));

    // Renaming keeps the selection on it; a taken name is refused.
    let mut back = edit.anchor_named(id, "Spine2").unwrap();
    back.name = "back".into();
    edit.set_anchor(id, "Spine2", back);
    assert_eq!(edit.anchor.as_ref().map(|p| p.name.as_str()), Some("back"));
    let mut clash = edit.anchor_named(id, "back").unwrap();
    clash.name = "anchor".into();
    edit.set_anchor(id, "back", clash);
    assert!(edit.anchor_named(id, "back").is_some());

    edit.remove_anchor();
    assert_eq!(edit.anchors(id).len(), 1);
    assert_eq!(edit.anchor, None);
    // Nothing went to other assets.
    assert!(edit.others.is_empty());
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
