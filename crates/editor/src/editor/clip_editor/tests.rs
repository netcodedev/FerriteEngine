use ferrite::core::model::clip_file::{BonePose, ClipFile, Transform};

use super::*;

fn pose() -> Transform {
    Transform {
        translation: [0.0; 3],
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: [1.0; 3],
    }
}

#[test]
fn playing_loops_or_stops_at_the_end() {
    let mut clip = ClipEdit::new(ClipFile::new(None, 1.0));
    clip.toggle_play();
    clip.advance(1.25);
    assert!(clip.playing);
    assert!((clip.time - 0.25).abs() < 1e-5);
    clip.looping = false;
    clip.advance(1.0);
    assert!(!clip.playing);
    assert_eq!(clip.time, 1.0);
    // Playing again at the end starts over.
    clip.toggle_play();
    assert_eq!(clip.time, 0.0);
}

#[test]
fn keys_go_to_the_playhead_and_can_be_deleted() {
    let mut clip = ClipEdit::new(ClipFile::new(None, 2.0));
    clip.set_time(0.5);
    clip.key(&[("arm".into(), pose()), ("leg".into(), pose())]);
    assert_eq!(clip.file.key_times(), vec![0.5]);
    assert_eq!(clip.keys, 1);

    // With a bone selected only its key goes.
    clip.select(Some("arm".into()));
    assert!(clip.delete_key());
    assert!(clip.file.track("arm").is_none());
    assert!(clip.file.track("leg").is_some());
    // Without, every key at the playhead.
    clip.select(None);
    clip.key(&[("arm".into(), pose())]);
    assert!(clip.delete_key());
    assert!(clip.file.tracks.is_empty());
    assert!(!clip.delete_key());
}

#[test]
fn the_timeline_lists_keyed_bones_in_skeleton_order_and_the_selection() {
    let mut clip = ClipEdit::new(ClipFile::new(None, 2.0));
    clip.key(&[("leg".into(), pose()), ("arm".into(), pose())]);
    clip.select(Some("head".into()));
    let order = vec!["arm".to_string(), "head".into(), "leg".into()];
    let view = TimelineView::of(&clip, &order);
    let bones: Vec<&str> = view.rows.iter().map(|(b, _)| b.as_str()).collect();
    assert_eq!(bones, vec!["head", "arm", "leg"]);
}

#[test]
fn saving_carries_the_rest_pose_along() {
    let mut clip = ClipEdit::new(ClipFile::new(None, 1.0));
    clip.rest.push(BonePose {
        bone: "hips".into(),
        pose: pose(),
    });
    assert_eq!(clip.to_save().rest.len(), 1);
    assert!(clip.file.rest.is_empty());
}
