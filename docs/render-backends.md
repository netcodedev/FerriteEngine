# Render backends: plan

Status (2026-11-20, branch `feat/render-backends`): stage A and B done
(only window/context creation and GL diagnostics still call GL outside
`core/gfx/gl_backend.rs`); stage C done in the form described under C
below; stage D in progress.

Goal: the engine renders through a backend-agnostic layer (`gfx`) with two
backends, the existing OpenGL renderer and a new Vulkan one (`ash`),
chosen at start-up, so the two can be compared frame for frame. The layer
is shaped so hardware ray tracing (Vulkan `VK_KHR_acceleration_structure`,
`ray_query`, `ray_tracing_pipeline`) can be added later without another
rewrite.

Non-goals for now: ray tracing itself, multithreaded command recording,
switching backends while running, macOS/Metal.

## Where we start

An inventory of the GL use (2026-11-20):

- ~1000 raw `gl::` calls in ~52 files: engine renderer and terrain (~925),
  editor (~83: outline, previews, stats, screenshots), engine-ui (~22:
  stencil clipping, popups). Game scripts, `game`, `server` and `mcp` call
  no GL.
- Drawing is immediate: every `Component::render` issues GL calls in the
  pass `Scene::render_passes_lit` runs. The pass is told through globals
  (`depth_pass::is_active`, the bound framebuffer via
  `framebuffer::bound_draw`, `ssao::in_depth_pass`, exposure/ground flags).
- Pipeline state is toggled ad hoc and often left changed; later code
  relies on it (see the inventory notes in this file's history).
- 32 vertex+fragment programs, 53 GLSL files, assembled in Rust by
  `concat!`/`format!`, ~200 loose uniforms set by name, samplers on fixed
  texture units 0–18, four buffer blocks (lights, cluster grid/ranges/
  indices). Game scripts supply terrain shaders at runtime
  (`TerrainSource::shaders`) and get the GL `Shader` in `load_textures`.
- GL conventions baked in: depth in [-1,1] (shader math in ssao, blur,
  water, particles, lighting cascades and cube shadows, weather maps,
  precipitation; Rust projections and unprojections), bottom-left origin
  (`gl_FragCoord`, viewport rects, `ReadPixels` flips), an 8× MSAA default
  framebuffer with stencil that is read back for screenshots.

## Decisions

1. **One layer, two backends, chosen at start-up.** `gfx` lives in the
   engine crate (`core/gfx`). The device is a backend enum
   (`Gl(GlDevice) | Vulkan(VkDevice)`) behind free functions on a
   thread-local, like today's global GL context; resources are typed
   handles (`BufferId`, `TextureId`, `SamplerId`, `PipelineId`,
   `ShaderId`, `RenderTargetId`) into slot maps. Selection:
   `--renderer vulkan|gl` / `FERRITE_RENDERER`, saved per project later.
   Default stays GL until Vulkan reaches parity.

2. **Vulkan 1.3 with dynamic rendering**, no `VkRenderPass` objects;
   `gpu-allocator` for memory; `ash-window`-style surface creation through
   GLFW's `vulkan` feature (window created with `ClientApi(NoApi)`).
   Two frames in flight. Deferred destruction of every resource by frame
   index (today only vertex arrays are deferred).

3. **Same conventions in both backends, so shaders and Rust math are
   shared:**
   - Depth range stays GL's [-1,1]: Vulkan enables
     `VK_EXT_depth_clip_control` (`negativeOneToOne`, supported by the
     NVIDIA and Mesa drivers here), so every projection, unprojection and
     the shader formulas that assume [-1,1] stay as they are. (Moving both
     to [0,1] with reversed Z is a later, separate step.)
   - Image rows: row 0 is NDC y = −1 in both APIs (with a plain,
     positive-height Vulkan viewport), and so is `gl_FragCoord`'s origin,
     so offscreen rendering, sampling and viewport rects agree without
     flipping. Vulkan sets the front face to clockwise to match GL's
     counter-clockwise, and flips once when the final image goes to the
     swapchain.
   - The "window" is a render target of its own (`RenderTarget::WINDOW`):
     GL's multisampled default framebuffer, and in Vulkan an offscreen
     target like it (colour at the window's sample count, depth-stencil),
     resolved and blitted (flipped) to the swapchain at present.
     Screenshots read it through `gfx`, so the MCP bridge works the same
     on both.

4. **Pipelines are objects.** A `PipelineDesc` (shader, vertex layout,
   topology, depth test/func/write, blend, cull, colour mask, polygon
   mode, target formats, sample count) is hashed and cached. GL applies
   the difference to the last state (and stops leaking state); Vulkan
   creates a `VkPipeline` (with a pipeline cache). Viewport and scissor
   are dynamic. Stencil ops for the UI clipping are part of the desc.

5. **Binding model** (planned as four sets; built as one push set, see
   stage C):
   - set 0 *frame*: frame UBO (camera, sun/moon, clouds, weather, SSAO,
     exposure…), lights UBO, cluster buffers, shadow maps, SSAO result,
     weather maps, cloud noise, loaded-columns map.
   - set 1 *pass*: pass UBO (view-projection, pass kind, …) and pass
     inputs (opaque scene colour/depth, cloud targets).
   - set 2 *material*: material UBO + textures (PBR maps, terrain arrays,
     impostor arrays).
   - set 3 *draw*: per-draw data larger than push constants (bones).
   Per-draw constants (model matrix, tint, flags; ≤128 bytes) are push
   constants in Vulkan and a UBO at a reserved binding in GL. Uniform data
   is written into a per-frame ring buffer (`gfx::frame_uniforms`).
   In GLSL this is written once with macros: `SET(s, b)` expands to
   `layout(set = s, binding = b)` for Vulkan and to
   `layout(binding = s * 16 + b)` for GL; `PUSH_CONSTANTS` likewise.
   Vulkan binds with push descriptors (`VK_KHR_push_descriptor`), which
   keeps GL's "bind this texture here, then draw" model and needs no
   descriptor pools; bindless material tables come with ray tracing.

6. **Shaders: one GLSL 4.60 source, compiled at runtime.** A small
   preprocessor resolves `#include "name"` against a registry of engine
   sources and script-supplied sources, adds defines and the stage, and
   checks interfaces. GL compiles the text as today (no SPIR-V needed,
   hot reload unchanged); Vulkan compiles it to SPIR-V with `shaderc`
   (linked statically from `libshaderc_combined.a`, so exported games
   need nothing extra). A test compiles every engine shader for both
   targets, so SPIR-V breakage shows up in `cargo test` long before the
   Vulkan backend draws. Ray tracing stages (`GL_EXT_ray_tracing`) need
   only new stage kinds later.

7. **Pass context is explicit.** `Scene` begins each pass with a
   `PassDesc` (kind: SunShadow, LightShadow, Exposure, Ground, Prepass,
   Opaque, Transparent, Overlay, Preview…; target; view-projection). The
   existing query functions (`depth_pass::is_active`, `hdr::draws_to_view`,
   `ssao::in_depth_pass`, `exposure::in_*_pass`) stay as a facade over
   it, so components and scripts keep working; nothing reads GL state to
   find out where it draws.

8. **Script API stays source compatible where it can.**
   `Component::render`/`render_transparent`/`render_overlay`,
   `Model::render`, `render_meshes`, `set_tint`, `generated::build_model`,
   `ScatterInstances::draw`, `LineRenderer`, `Text`, `Plane`,
   `Texture::new/load_material_map/share` keep their signatures.
   `TerrainSource::shaders()` keeps returning GLSL, now using the
   include/macro conventions; `load_textures(&Shader)` becomes
   `load_textures() -> TerrainTextures` (per-layer parameters go into the
   terrain material UBO instead of `set_uniform` calls). The demo is
   updated in the same change.

9. **Ray tracing readiness, without building it yet:**
   - Meshes become engine resources (`MeshId`: vertex + index buffers,
     layout, bounds) created through `gfx`, with a `keep_for_rt` usage
     flag (Vulkan: `SHADER_DEVICE_ADDRESS` +
     `ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY`), so a BLAS can be
     built from GPU buffers without keeping CPU copies.
   - Every opaque draw goes through one submission function
     (`gfx::draw_mesh(mesh, material, transform, …)`), which is where a
     later frame collector gathers TLAS instances.
   - Materials become bindless-friendly (material index + texture
     indices), which is what hit shaders need.

## Stages

Each stage ends with the editor running the demo and a screenshot
comparison against the stage before.

### A. `gfx` core on GL
- `core/gfx`: device enum, handles and slot maps, buffers (static,
  dynamic, stream/ring), textures (2D, 2D array, 3D, cube array; all
  formats in use; mips, sub-updates), samplers (incl. compare), render
  targets (colour/depth attachments, layers, MSAA, resolve, blits),
  pipelines and the state cache, the encoder (`begin_pass`,
  `set_pipeline`, `bind`, `push_constants`, `draw*`, `end_pass`),
  timestamp queries, readback, deferred destruction.
- Window target and present.
- Shader preprocessor with `#include`, defines and stage injection;
  `Shader` keeps `set_uniform_*` as a GL-only shim for now so the port can
  be done file by file.

### B. Port everything onto `gfx`
In dependency order: shader/vertex arrays/textures/framebuffers →
models and the PBR path → terrain, water, scatter, far scatter,
impostors → sky, HDR, SSAO, shadows, lights → weather (exposure,
shelter, snow, snow layer, precipitation), particles → line, plane,
text, texture renderer, debug views → engine-ui (stencil clip) →
editor (outline, previews, stats, screenshots). Afterwards no file
outside `core/gfx/backend/gl` calls `gl::`. Explicit `PassDesc`
replaces the global pass flags.

### C. Uniform blocks and SPIR-V
As built (it differs from the first plan, for less risk and an
unchanged GL baseline): the GLSL sources stay as they are, and the GL
backend compiles them as before. For Vulkan, `gfx::glsl` rewrites a
program's two stages together: all loose uniforms of both stages
become one std140 block (`FerriteParams`) whose layout it computes, so
the by-name setters (`Device::set_uniform`, which the GL backend does
with `glProgramUniform*`) write into it; loose samplers get bindings,
mapped at draw time to the texture bound on the unit the program set
for them; buffer blocks keep their slot. Everything is in descriptor
set 0, filled with push descriptors (one push set per pipeline is all
Vulkan allows; 32 descriptors are enough). `FERRITE_SHADER_DUMP=<dir>`
writes every program's sources; an ignored test rewrites and compiles
them all with `glslc` for Vulkan 1.3. Hand-designed per-frame/per-pass
blocks and push constants remain an optimisation for later.

### D. Vulkan backend
Instance/device (with validation layers in debug builds), swapchain,
frames in flight, `gpu-allocator`, the `gfx` resource and encoder
implementation, pipeline cache, timestamps, readback, present with the
flip. Then pass by pass to parity with GL, comparing screenshots.
Editor statistics show the backend and its GPU/CPU frame times.

As built (`core/gfx/vulkan`, feature `vulkan`, `FERRITE_RENDERER=vulkan`;
GL stays the default and is fallen back to when Vulkan cannot start):
- Commands are recorded in call order into one command buffer per frame;
  rendering begins lazily at the first draw or clear of a pass and is
  ended by copies, blits, readback, texture uploads and layout changes of
  sampled textures. Buffer uploads go into a command buffer of their own
  that runs before the frame's. Every rendering begin and transfer is
  preceded by a global memory barrier (to be made finer).
- Multisampled depth is resolved by a render pass that only resolves
  (sample 0), as copies and blits cannot read it.
- GLSL varyings are linked by name in the SPIR-V (shaderc numbers each
  stage on its own); `gl_Position` is invariant; multisampling uses GL's
  sample positions (`VK_EXT_sample_locations`).
- `FERRITE_VK_VALIDATION=1` (default in debug builds) turns the
  validation layer on; its messages go to the log.
- Left over against GL: alpha-tested foliage covers slightly fewer pixels
  (isolated pixels across tree crowns), cause not found yet.

## Verification

- `cargo test` (engine, scripts) green at every commit.
- Reference frames: the editor captures the demo scene from fixed
  editor cameras (`FERRITE_CAPTURE`), with the clock frozen, before
  stage A; after each stage the same frames are captured and compared
  per pixel (small tolerance for MSAA/precision; differences shown as
  images, not judged from numbers alone).
- Vulkan: no validation errors in a session through the demo (walk,
  drive, night, snow, editor previews).

## Open questions (for later, not blocking)

- [0,1] depth with reversed Z for the far terrain (precision), in both
  backends at once.
- Whether the GL backend should eventually also consume SPIR-V
  (`ARB_gl_spirv`) instead of GLSL text.
