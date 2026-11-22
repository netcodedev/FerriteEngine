# Temporal anti-aliasing and motion vectors

Status (2026-11-22): built on branch `feat/motion-vectors`, off by default.

## Why

MSAA smooths the edges of triangles, not what a shader cuts out of them:
leaves, grass and other alpha-tested cards stay jagged and flicker in
motion. Later work needs the same groundwork TAA does: the ray traced
shadows' denoiser blends noisy samples over frames, and DLSS takes a
jittered frame with motion vectors. Both need to know, for every pixel,
where its surface was a frame before.

## What it does

The graphics setting **"Temporale Kantenglättung (TAA)"** (`display.taa`,
off at every preset) switches it on. Then, for the camera's view:

1. The view is drawn **without MSAA**, with its projection shifted by a
   sub-pixel offset that changes every frame (Halton 2,3, eight frames;
   `taa::jitter_offset`). `Hdr::view_projection` hands the scene the
   jittered matrix; culling keeps the plain one.
2. After the scene is drawn and resolved, the **motion vectors** are
   written (`Rgba16F`: uv motion, the surface's distance then and now):
   - for the whole view from the depth and the camera of this and the
     last frame (`velocity.glsl`; the reprojection matrix is worked out in
     f64, as the far plane is kilometres away);
   - over that, for **models that moved** since the last frame (rigid and
     skinned: vehicles, characters, an object carried along). A model
     records each mesh it draws in the view with its transform and
     bones (`taa::motion::History`); a mesh that moved is drawn again into
     the motion vectors, with this and the last frame's transform, where
     the view's depth shows it.
3. **Resolve** (`resolve.glsl`) blends the frame into the history found
   through the motion vectors: Catmull-Rom history, clipped to the colours
   around the pixel (YCoCg, mean ± 1.25 σ), bright samples weighted down,
   about 10 % of the new frame per frame (more for fast motion). The
   history is dropped where it shows a surface at another distance than
   the pixel's surface had a frame before (what a moving object revealed),
   and when the view's size changes.
4. Bloom, exposure and tone mapping work on the result as before. 2D
   overlays, gizmos and editor lines are drawn after it, unjittered.

With TAA off nothing of this runs: the view is drawn as before (MSAA per
setting), and the picture is the same as without this change.

## Limits

- Motion vectors of moving things other than models (particles, rain,
  water waves, wind sway of leaves) are the camera's: the history clipping
  keeps them from smearing, at the price of a little less smoothing there.
- A mesh drawn more than once in a view (the same model at two places)
  has no single last position and keeps the camera's motion.
- TAA works at the view's resolution; with a resolution scale below 100 %
  the result is scaled up afterwards as before (no temporal upscaling yet).

## Files

`core/renderer/taa/` (`mod.rs`, `motion.rs`, shaders), `hdr/mod.rs`
(targets, order of the passes), `model/model.rs` (`record_motion`),
`graphics/defs.rs` (setting).
