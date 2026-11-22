# Ray traced sun shadows

Status (2026-11-22): built on branch `feat/rt-shadows` (on top of
`feat/motion-vectors`), off by default.

## What it does

The graphics setting **"Raytracing-Schatten"** (`shadows.raytraced`, group
"Licht", off at every preset) replaces the sun's shadow cascades with ray
traced shadows where the device can trace rays (Vulkan with
`VK_KHR_ray_query` and `VK_KHR_acceleration_structure`; the log says
"Vulkan: ray tracing (ray queries) available"). Elsewhere, and with
OpenGL, the cascades stay. `FERRITE_NO_RAY_TRACING=1` turns ray tracing off
for testing.

Each frame:

1. **Collect** (`renderer/raytracing`): the scene is drawn once more as a
   depth pass from above around the camera (as far as terrain is loaded,
   plus 128 m), into a 1×1 target. Models, terrain chunks and landscape
   scatter do not draw then but hand their meshes over. Each mesh gets a
   bottom level acceleration structure once (kept while it is used), each
   placed mesh an instance in this frame's top level structure, which is
   built anew every frame.
2. **Depth prepass**: runs whenever the setting is on, as it does for SSAO.
3. **Trace** (`trace.glsl`): one ray per pixel from the visible surface
   toward a random point on the sun's disc (the moon at night, as the
   shadow map would be). Opaque meshes stop the ray; cut-out materials
   (leaves) let 45 % of the light through each.
4. **Denoise**: blended into the frames before, moved with the camera
   (`accumulate.glsl`, history dropped where it showed another surface,
   clamped to the neighbourhood), then a 5×5 blur within a surface
   (`filter.glsl`).
5. The lit shaders read the result per pixel in `sunShadow`
   (`pbr/lighting.glsl`) instead of the cascades.

## Limits

- **Leaves are not alpha-tested**: a leaf card stops a share of the light
  across its whole quad, so tree shadows are soft blobs without the leaf
  pattern the cascades show. Alpha-testing needs the leaf textures in the
  trace shader (bindless textures); the next step if the shadows should
  keep their detail.
- Skinned models (characters) cast no ray traced shadows yet: their
  structure would have to be refit each frame.
- Detail scatter (grass) casts none: too many instances for what it adds.
- Water and particles take the shadow of the surface behind them.
- The top level structure is rebuilt every frame; with many instances that
  costs CPU and GPU time. Not measured on a real GPU yet.
- Vegetation sways in the wind; the rays see it at rest.

## Files

`core/gfx/vulkan/accel.rs` (acceleration structures), `core/gfx/glsl.rs`
(`accelerationStructureEXT` bindings), `core/renderer/raytracing/`
(collecting, trace, denoise), `core/scene/scene.rs` (order of the passes),
`collect_for_ray_tracing` in `model/model.rs`, `terrain/chunk.rs`,
`terrain/scatter/instances.rs`.
