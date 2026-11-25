# Ray tracing

Four graphics settings, independent of each other: the sun's shadows
(below), the lamps' shadows ("Ray traced lamp shadows"), the ambient
occlusion ("Ray traced ambient occlusion") and the reflections ("Ray traced
reflections"). They share one collected
scene: one top level structure, one cut-out atlas.

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
   The ground does not shadow itself within 2.5 of its grid steps along
   the ray (the step it is meshed at there: 1, 2, 4 or 8 m by distance,
   `terrain::chunk::ground_steps_from`): a ray that meets it that close
   goes on past it (`traceShadowPastGround`, at most three times). It is
   drawn smooth, with blended normals, but its facets are flat; where the
   sun is low over rough ground they shadowed each other in sharp-edged
   patches (the "shadow terminator", found on the rocky hills far from
   the origin). Things standing on the ground, and ground further off (a
   hill, a ridge), still cast their shadows.
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

# Ray traced lamp shadows (ReSTIR DI)

Status (2026-11-23): built on branch `feat/rt-light-shadows`, off by
default.

## What it does

The graphics setting **"Raytracing-Lampenschatten"**
(`shadows.raytraced_lights`, group "Licht", off at every preset) replaces
the point and spot lights' shadow cube maps with ray traced shadows where
the device can trace rays (as for the sun). The cube maps are then not
drawn at all, and every lamp with "shadows" on gets them, not only the
eight nearest. Lamps have a size now, **radius** (metres, 0–0.5, default
0.05, saved in scenes; older scenes get the default): the shadows soften
with it. The cube maps ignore it.

The lit shaders loop over the lights per fragment, so a fullscreen pass
cannot give them one visibility per lamp. It gives them a share per pixel
instead (Heitz et al. 2018, "Combining analytic direct illumination and
stochastic shadows"): the light of the shadow casting lamps that gets past
what is in the way, divided by what would arrive with nothing in the way,
per colour (a red lamp in shadow next to a blue one in the open keeps its
colours). `localLights` (`pbr/lighting.glsl`) sums those lamps without
shadow and multiplies the sum by it; lamps without shadows are added as
before.

Each frame, after the depth prepass, five fullscreen passes (RGBA16F,
`raytracing/lamps.rs`):

1. **Candidates + temporal** (`restir_initial.glsl`): position and normal
   from depth. 4 lamps drawn uniformly from the pixel's cluster list
   (`lightsAt`), one kept by weighted reservoir sampling in proportion to
   the light it would give there (`lampTarget`: luminance of radiance ×
   range fade ÷ d² × cone × N·L, 0 for lights without ray traced shadows).
   Last frame's reservoir at the reprojected pixel (same surface by
   distance) is merged in, its M capped at 20 × the candidates, its lamp's
   light looked at anew here. A reservoir: lamp index, W, M, distance.
2. **Spatial** (`restir_spatial.glsl`): 4 neighbours within 16 px on the
   same surface merged in (M capped at 16 each). Also next frame's
   history.
3. **Trace** (`light_trace.glsl`): one ray toward a random point on the
   chosen lamp's disc (its radius), ending 5 cm short of it (its own bulb
   or glass does not count); opaque geometry and alpha-tested cut-outs
   stop it as for the sun (`rays.glsl`). Share = that lamp's light ×
   visibility × W ÷ all shadow casting lamps' light here.
4. **Accumulate** (`light_accumulate.glsl`): blended into the frames before
   (12 % this frame), clamped to the neighbourhood widened by 0.1.
5. **Filter** (`light_filter.glsl`): 5×5 blur within a surface, clamped to
   0–1, bound to unit 26 for the lit shaders.

Debug view: **"Raytracing-Lampenschatten"** in the Performance panel
(MCP: `editor_set_debug_views` `lamp_shadows`) shows the share over the
view: white where all the lamps' light arrives (and where none reaches),
black in full shadow, coloured where lamps of different colours are
blocked differently.

Shared code: `pbr/lights.glsl` (the light struct, the buffers, clusters,
`rangeFade`, `spotCone`, `castsShadows`) is part of the lit shaders and
of the ReSTIR passes; `raytracing/surface.glsl` (positions and normals
from depth, noise, random numbers) and `raytracing/rays.glsl` (cut-out
records, alpha test, `traceShadow`) are shared with the sun's trace.
Light packing: `cone.w` is the lamp's radius when its shadows are ray
traced, −1 otherwise.

## Measured

Demo, RTX 4070 Ti, 2536×1390 viewport, campfire and one test lamp at
night: the five passes cost about 1.1 ms GPU (the cube maps they replace
0.2–0.6 ms there). Their cost hardly grows with the number of lamps (one
ray per pixel); the cube maps' grows with every lamp, up to eight. A
lamp 4 m above the pickup throws the same shadow outline as with cube
maps; with a radius of 0.4 its edges soften.

## Limits

- The share approximates: diffuse and specular light share one share per
  pixel, and the normal it is worked out with comes from depth.
- Water takes the share of the surface behind it. Particles, rain and
  snow get the lamps' light unshadowed (`localLightAt`): with the share of
  the ground behind, smoke over a campfire went dark against the meadow.
- Grass casts lamp shadows only with "Raytracing-Schatten von Gras", as
  for the sun. Landscape scatter (tufts) does cast them: a lamp low over
  a meadow throws long streaks of tuft shadows, as it should.
- With many lamps reaching one pixel, a moving shadow takes a few frames
  to settle (temporal accumulation); one lamp's moving shadow is sharp
  only after the history catches up.
- The passes run at full resolution even where no lamp reaches; half
  resolution or skipping empty tiles would save most of the 1.1 ms.
- Editor pictures (thumbnails, impostor bakes) drawn while the setting is
  on see no lamp shadows (there are no cube maps then).
- Not tested on OpenGL or with `FERRITE_NO_RAY_TRACING=1` in the editor;
  there the setting is skipped and the cube maps are used (same check as
  for the sun).

## Files (lamps)

`core/renderer/raytracing/lamps.rs`, `restir_initial.glsl`,
`restir_spatial.glsl`, `light_trace.glsl`, `light_accumulate.glsl`,
`light_filter.glsl`, `lamps.glsl`, `surface.glsl`, `rays.glsl`;
`pbr/lights.glsl`, `pbr/lighting.glsl` (`rayTracedLamp`,
`lampShadowRatio`); `light/light.rs` (radius, packing);
`scene/scene.rs` (`wants_ray_traced_lamps`, the order of the passes);
`renderer/debug_views.rs`.

# Ray traced ambient occlusion

Status (2026-11-23): built on branch `feat/rt-ao`, checked in the editor
(demo, by the house and pickup at noon): the debug view "Ambient
occlusion" shows contact darkening under the porch beams, the pickup and
in the house's corners where SSAO shows it, plus what SSAO misses at the
screen's edges and behind things. Cost there (2536×1390, RTX 4070 Ti):
SSAO about 0.2 ms, RT-AO about 0.7 ms GPU.

## What it does

The graphics setting **"Umgebungsverdeckung"** (`effects.ao`, group
"Licht") is a choice: `off` (preset Low), `SSAO` (Medium and up) or
`RT-AO`. With RT-AO, where the device can trace rays, SSAO is not worked
out; elsewhere SSAO is used. The sky light's own switch, radius and
intensity apply to both. The samples knob `effects.ssao_samples` only sets
SSAO's quality now (8 at Low, so SSAO picked by hand at Low works).

Each frame, after the depth prepass (`raytracing/ao.rs`):

1. **Trace** (`ao_trace.glsl`): 2 rays per pixel, cosine weighted into the
   hemisphere above the surface (normal from depth), as long as the AO
   radius; cut-outs alpha-tested as for the shadows. A hit darkens by
   1 − (t / radius)², so what is near darkens most; the result is raised
   to the AO intensity, as SSAO does.
2. **Denoise** (`denoise.rs`, shared with the lamps' shadows):
   `light_accumulate.glsl` (10 % this frame) and `light_filter.glsl`.
3. Bound to `SSAO_UNIT` with the AO origin set: the lit shaders
   (`screenOcclusion`) use it unchanged. The debug view "Ambient
   occlusion" shows it while RT-AO is on.

## Limits

- Scatter is collected a level of detail coarser than it is drawn (far
  trees are drawn as impostor cards): a ray starting on the drawn bark
  met the thicker collected trunk at once (black trunks, speckled leaves
  in the background). So each ray skips its first 0.6 % of the distance
  to the camera, and the occlusion fades out from 80 to 150 m, as SSAO's
  does. Close up, scatter darkens a little less than it would with
  matching geometry.
- Grass (detail scatter) occludes only with "Raytracing-Schatten von
  Gras"; SSAO darkens under grass from the screen.
- Water and particles take the occlusion of the surface behind them, as
  with SSAO.

# Ray traced reflections

Status (2026-11-23): built on branch `feat/rt-reflections`, checked in the
editor (demo, at the pickup at noon, dry and with rain puddles).

## What it does

The graphics setting **"Reflexionen"** (`effects.reflections`, group
"Licht") is a choice: `Sky` (every surface reflects the sky alone, water
too), `SSR` (every preset: water marches the scene in screen space, as
before) or `RT`. With RT, where the device traces rays, every surface
smoother than roughness 0.3 and the water trace their reflection; from
0.1 down it is all traced, between 0.1 and 0.3 it fades into the sky. So
puddles (0.03), glass (0.05) and chrome (0.15) mirror the scene, the wet
film on ground (0.32) and car paint (0.40) keep the sky (tracing the wet
film too cost 6 ms in the rain).

Unlike the other stages this is no fullscreen pass: the renderer is
forward and only the lit shaders know a pixel's roughness and normal, so
they trace themselves (inline ray queries in `environment`,
`pbr/lighting.glsl`, and in `water_fragment.glsl`). `gfx::Program` puts
the ray query extensions and `#define RAY_QUERIES` in front of every
fragment stage that asks (`#ifdef RAY_QUERIES`) where the device traces
rays; elsewhere (OpenGL, no ray tracing) that code drops out.

A reflection ray (`traceClosest`, `raytracing/rays.glsl`) finds the
nearest opaque or solid cut-out triangle and works out what
is there from the instance's **surface record** (every instance has one
now, `SurfaceRecord`: buffers, cut-out, look): the triangle's normal, the
colour (a texture from the new **albedo atlas**, 128 layers of 256²
sRGB, times the material's base colour; the ground's vertex colour;
skinned meshes their base colour) and the emission. `shadeHit`
(`pbr/reflections.glsl`) lights it: the sun or moon past what is in the
way (a second ray), the sky, the lamps without shadow, its emission.
Where the ray meets nothing, the sky. Mirrors (roughness under 0.1)
look 16 km, out to the horizon's edge; blurrier surfaces 300 m (water
too). Hits are hazed with their distance from the mirror as the terrain
is with its distance from the camera (`reflectionHaze`, same numbers as
`terrain/shaders/haze.glsl`).

The **horizon** (the ground beyond the chunks) is in the rays' scene for
reflections only: its instance has its own mask (`MASK_FAR`), which the
shadow and occlusion rays leave out (it lies 4 m under the chunks' ground
and is coarse, so it would shade valleys and tunnels), and its record is
marked `far`: a reflection ray takes a hit on it only outside the loaded
chunks' circle (`set_far_hole`, uniform `farGroundHole`), where it is
drawn.

Where a ray meets the ground, its colour comes from **ground maps**
(`raytracing/ground.rs`): the terrain drawn from straight above with its
own program into a texture, its colour instead of its light
(`albedoBake`: `shadeSurface` hands back the albedo; the bake counts as a
view in linear colour). So reflections show the real ground (roads,
paths, soil, grass, wet and snowy as it is) for any world, whatever its
program does. Three levels of 2048² (sRGB): a near one 256 m across
(12.5 cm a texel), used for hits within 60 m of the ray's start, a far
one over the loaded ground (about 0.8 m a texel) and one over the whole
horizon (32 km across, 16 m a texel; the horizon is drawn into the maps,
its far scatter not). The near and far ones are drawn again when the
camera has moved an eighth of them, the horizon one every 1024 m; and
every 90, 300 or 900 frames for the weather and time of day. Past them
the ground's vertex colour stands in (in the demo a plain green, so it
must not show). A world's own terrain program must leave its haze out
while `albedoBake` is 1 (the engine's `terrainHaze` does): the far levels
lie kilometres from the camera and would turn to the sky's light, which
is black in the bake. Debug view: **"Boden-Karten (Reflexionen)"** (MCP
`ground_maps`) shows the three along the bottom. Vertical faces and overhangs (cliffs, caves) are
not in a map from above: there the colour of the ground above or below
is used.

How rays see a material follows how it is drawn (`ray_visibility`): an
opaque one whole, a cut-out with an opacity texture tested against it,
one without by its colour's alpha (the pickup's glass, alpha 0.35 under
the cut-off 0.5, is cut away, so rays pass it and the rear-view mirror
looks through the rear window).

## Measured

Demo, RTX 4070 Ti, 2536×1390: at the pickup in dry weather the opaque
pass costs about 0.3 ms more than with SSR (chrome and glass trace);
with wetness and puddles at 1 everywhere about 2.6 ms more.

## Limits

- One sharp ray per pixel, no denoising: surfaces between roughness 0.1
  and 0.3 get a sharp reflection faded with the sky, not a blurred one.
- Hits are lit without ambient occlusion and without the lamps'
  shadows; normal maps and roughness at the hit are not looked at.
- Textures in the albedo atlas are 256²; a graph that moves the UV in
  other ways than the standard scale and offset is not followed.
- Meshes whose materials cast no shadows are not in the scene the rays
  see, so they do not show in reflections either.
- Not tested on a lake or river yet (none near the demo's start).
- The pickup's mirrors (demo, `tools/pickup_model.py`, material
  `Mirror`: metal, roughness 0.02) are aimed at the `driver_eye` anchor;
  with "Reflexionen" SSR or Sky they show the sky only.

## Files (reflections)

`pbr/reflections.glsl`, `pbr/lighting.glsl` (`environment`),
`terrain/shaders/water_fragment.glsl`, `raytracing/rays.glsl`
(`traceClosest`), `raytracing/albedo.glsl`, `raytracing/mod.rs` (`Look`,
`SurfaceRecord`, `Atlas`), `gfx/mod.rs` (`with_ray_queries`),
`raytracing/ground.rs` (ground maps).
