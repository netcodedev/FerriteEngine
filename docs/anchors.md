# Anchors and clip references

Many animations only make sense against something else: a swing of an axe
needs the axe in the hand, sitting down needs the seat of the truck. The
clip editor shows such *references* next to the model being animated, and
the points where two models meet are *anchors* saved on the model assets,
so the game can use the same points the animation was made against.

## The two cases

1. **The animated model carries the reference** (an axe in the hand, a
   backpack on the back). The reference follows a bone of the animated
   model as the clip moves it.
2. **The reference carries the animated model** (a character in the
   driver's seat of a truck). The reference stands still and the clip
   moves the body against it.

Both are the same pairing of two anchors, one on each model: the
reference is drawn so that its anchor lies on the model's anchor. What
differs is which of the two moves with the clip:

| Link | The reference is placed by |
| --- | --- |
| *Held in the right / left hand* | its grip (`ModelSettings::grip`), in the hand of the posed model, exactly as `HeldItem` holds it in the game |
| *Follows the model* | the model's anchor in the **current pose** (an anchor on a bone moves with the bone) |
| *Carries the model* | the model's anchor in the **rest pose**: the reference stands still while the clip moves the body |

## Anchors

An anchor is a named frame on a model asset, saved in its sidecar
(`ModelSettings::anchors`):

```ron
anchors: [
    (name: "driver_seat", position: (-0.38, 0.72, 0.04), rotation: (0.0, 0.0, 0.0)),
    (name: "back", bone: Some("mixamorig1:Spine2"), position: (0.0, 0.1, -0.15), rotation: (0.0, 0.0, 0.0)),
]
```

- `position` is in metres, as the model is drawn (after its import scale),
  `rotation` in degrees about x, then y, then z.
- Without `bone` the frame is in the model's own space. With `bone` it is
  relative to that bone (its position and turn, not its scale), so it
  moves with the bone.
- No anchor at all means the model's origin.

`ferrite::core::model::anchor` has the maths the game uses as well:
`find_anchor(asset, name)` reads an anchor from a model asset,
`anchor_frame(model, transform, &anchor)` is its world frame on a posed
model, and `place_on(&frame, &anchor.local())` is the transform that puts
a model's anchor on a frame.

## In the clip editor

- The inspector has a **References** section: *Add reference* adds the
  first other model; each reference has its model, the link, its anchor
  and the model's anchor (dropdowns), and a remove button. References are
  saved in the clip file (`ClipFile::references`) and only matter to the
  editor.
- Anchors of both models are drawn as small axis crosses (red x, green y,
  blue z). A click selects one; the selected anchor shows rings to turn it
  and a diamond to move it on a plane facing the camera, like the bones.
  Its name, bone, position and rotation can be typed in the inspector's
  **Anchor** section, which also adds anchors (on the selected bone, or to
  a reference) and removes them.
- Moving an anchor edits the model asset it belongs to, which may be the
  reference's: **Save** writes the clip and every model whose anchors
  changed, **Revert** throws all of it away.
- A reference is moved by clicking it (its anchor is selected; one placed
  by its origin gets an anchor `attach` there) and dragging the selected
  anchor's arrows (along one axis) or its diamond (freely), or the model
  itself with **Shift** held. A plain drag always orbits the camera, also
  with the camera inside a model.
- Each reference and the clip's model have an **Opacity** slider (saved
  with the clip), to see the body through a door. The right or middle
  mouse button moves the view; *Center view* looks at the model again.
- Keys are only set by posing bones; moving an anchor or a reference
  changes no key.

The model inspector (outside clips) lists a model's anchors with the same
fields, to make or fix them without an animation.

## Requirements (checked by tests)

- R1: an anchor without bone is the model's own space: its frame is the
  model's transform times `Anchor::local`.
- R2: an anchor on a bone follows the bone: turning the bone turns and
  moves the frame; the bone's scale (e.g. the 0.01 of a Mixamo rig) does
  not shrink the anchor's offset.
- R3: `place_on(frame, anchor)` puts the model's anchor exactly on the
  frame (position and turn).
- R4: *Carries* places the reference by the rest pose: posing the model
  does not move the reference.
- R5: old clip files and sidecars without anchors or references load
  unchanged and are written back without the new fields.
- R6: saving a clip saves the changed anchors of other assets too; revert
  drops them.
