# Ray traced sun shadows

Status (2026-11-22): built on branch `feat/rt-shadows` (PR #82), off by
default.

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
   placed mesh an instance in this frame's top level structure. That
   structure is left as it is when nothing changed, refit in place when
   only transforms changed (at most 30 times in a row), and built anew
   when instances came or went. Skinned meshes (characters) are skinned
   on the CPU per pose, as `vertex.glsl` does, and each pose gets its own
   bottom level structure (kept two frames). With **"Raytracing-Schatten
   von Gras"** (`shadows.raytraced_grass`, off at every preset) detail
   scatter within 40 m is collected as well.
2. **Depth prepass**: runs whenever the setting is on, as it does for SSAO.
3. **Trace** (`trace.glsl`): one ray per pixel from the visible surface
   toward a random point on the sun's disc (the moon at night, as the
   shadow map would be; the sun's real angular size, so shadows soften
   with distance). Opaque meshes stop the ray. Cut-out materials (leaves)
   are alpha-tested as when drawn: the ray looks up the hit's UV in the
   mesh's own buffers (device addresses, `GL_EXT_buffer_reference`) and
   the material's opacity in an atlas (`opacity.glsl`: the texture the
   graph's opacity comes from, `Material::opacity_texture`, one 256×256
   layer each, 64 layers). The atlas and the layers it gives out are
   shared by every scene (the editor makes a new one after play mode);
   when all are taken, a layer unused for 120 frames goes to the next
   texture. A cut-out whose texture is not loaded yet, or finds no free
   layer, lets 45 % of the light through instead.
   With **"Raytracing-Schatten im Wind"** (`shadows.raytraced_wind`, off
   at every preset) the light through the leaves moves with them: a
   cut-out whose material sways (`Material::wind_sway`: the wind node's
   strength, flutter and bend height where they are fixed numbers, masks
   that vary counted as 1, times the scatter output's wind) carries them
   in its record. At a hit the trace works out the wind's offset there as
   `materialWind` does (the pivot from the instance's transform, the
   normal from the triangle), moves the point back to where the swayed
   leaf would be met, and looks up the opacity at that point's UV in the
   leaf's plane. In the demo about 0.1 ms on an RTX 4070 Ti.
4. **Denoise**: blended into the frames before, moved with the camera
   (`accumulate.glsl`, history dropped where it showed another surface,
   clamped to the neighbourhood), then a blur within a surface
   (`filter.glsl`) as wide as the penumbra there: the trace also writes
   how far along the ray the blocker was, the filter averages that over
   the neighbourhood and works out the penumbra's width on screen from
   the sun's size, so contact shadows stay sharp and shadows of far
   blockers soft (sigma 0.5–3 px).
5. The lit shaders read the result per pixel in `sunShadow`
   (`pbr/lighting.glsl`) instead of the cascades.

## Limits

- Leaves are tested against their texture at 256×256 and the UV the
  mesh has; a graph that moves or scales the UV before sampling is not
  followed.
- Skinned models are skinned on the CPU and get a new bottom level
  structure for every pose; fine for a few characters, costly for crowds.
- Grass casts shadows only with its own setting and only within 40 m:
  dense grass means many instances.
- Water and particles take the shadow of the surface behind them.
- While terrain streams in, the top level structure is built anew each
  frame. Not measured on a real GPU yet.
- Vegetation sways in the wind; the rays see it at rest. With the wind
  setting the pattern inside leaf cards moves, but the cards' outlines,
  trunks, branches and opaque leaves stay where they stand at rest.

## Files

`core/gfx/vulkan/accel.rs` (acceleration structures), `core/gfx/glsl.rs`
(`accelerationStructureEXT` bindings), `core/renderer/raytracing/`
(collecting, trace, denoise), `core/scene/scene.rs` (order of the passes),
`collect_for_ray_tracing` in `model/model.rs`, `terrain/chunk.rs`,
`terrain/scatter/instances.rs`.
