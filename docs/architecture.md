# Architecture

Ferrite is a Cargo workspace of several small crates. The split is not
arbitrary: it keeps the exported game (which must run on machines
without Assimp, GLFW or OpenGL development headers installed) separate
from the editor, and lets scripts link the engine as a shared library
without each build ending up with its own copy.

## Crate map

```
crates/
├── engine            ferrite        — the engine itself
├── engine-macros     ferrite_macros — the #[derive(Script)] macro
├── engine-dylib      ferrite_dylib  — links the engine as a dylib
├── engine-ui         ferrite-ui     — retained-mode UI library
├── editor            ferrite-editor — the scene and animation editor
├── scripts           ferrite_scripts— the open project's scripts, built as a dylib
├── sandbox           sandbox        — demo world
├── server            ferrite-server — headless development server
├── game              ferrite-game   — exported game template
└── mcp               ferrite-mcp    — authoring-time MCP server (NOT shipped in games)
```

### Why there is `engine-dylib`

`ferrite_dylib` is the engine as a shared library (`pub use ferrite::*`
with `crate-type = ["dylib"]`). Everything that depends on it links the
engine dynamically, so the editor and the script library it loads at
runtime share one copy of the engine: one set of GL function pointers, one
asset database, one script registry.

It is excluded from the workspace and only ever built as a dependency:
cargo builds a dylib dependency with `-C prefer-dynamic` and a workspace
member without, both would be written to the same unhashed file, and the
variant with a static `std` cannot be linked against.

For the same reason the scripts must be built against the very engine
library the editor runs with. The editor builds them in the engine's
workspace (`ferrite_scripts`), with the variables `cargo run` set for the
editor removed, so the build reuses that library instead of rebuilding it.

### What a game is

An exported game is a tiny `main` (template at `crates/game`) that
registers its scripts and calls `ferrite::core::runtime::run`. The
engine builds it in `standalone` mode, which statically links Assimp
and builds GLFW from source, so the resulting binary does not need
either of those installed on the player's machine.

## Engine layout

```
crates/engine/src/
├── core/
│   ├── application   the main loop, layers, delta time
│   ├── assets        the asset cache (models, textures, animations)
│   ├── camera        the 3D camera and projection
│   ├── entity        entities, components (camera, model, shape, …)
│   ├── export        building an exported game
│   ├── headless      running without a window (for the server)
│   ├── mcp_bridge    the editor's live bridge for the MCP server
│   ├── model         3D models, bones, animations, pose, outline, IK
│   ├── mouse_picker  world-space clicks in the viewport
│   ├── net           multiplayer: players and per-channel world events
│   ├── physics       the Rapier wrapper, rigid bodies, colliders
│   ├── primitives    2D layout types for the UI (position, size, offset, region)
│   ├── project       the .fp project file and its folders
│   ├── renderer      everything that touches OpenGL
│   │                  ├── framebuffer (shadow maps, debug maps)
│   │                  ├── hdr (ACES / filmic tone mapping, bloom, exposure)
│   │                  ├── light (sky light, cascaded shadow maps, scene
│   │                  │         lights, moon and bounce light)
│   │                  ├── pbr (shading)
│   │                  ├── shader (compile / link, VAOs, attributes)
│   │                  ├── sky (cumulus, altocumulus, cumulonimbus)
│   │                  ├── ssao (ambient occlusion)
│   │                  ├── texture (upload, mipmaps, array textures)
│   │                  └── weather (rain, snow cover, footprints, wet surfaces)
│   ├── runtime       opens a project and runs its startup scene
│   ├── scene         the scene, its scene file, deferred work, viewport
│   ├── script        the script registry and the dylib loader
│   ├── utils         DataSource (observable value) and other small helpers
│   ├── view_frustum  AABB vs clip-volume culling
│   └── window        GLFW window and input
├── player/           the Player component and its animation graph
└── terrain/          voxel, marching cubes, dual contouring, scatter, materials
```

## How a scene runs

`Scene::update` (play mode):

```
Scene::update
├── net::update            messages in and out, other players moved
├── physics step           Rapier, when physics is on
├── every entity's update  its components (scripts too), then its children
└── deferred work          what components queued with Scene::defer
```

In edit mode the editor calls `Scene::update_in_editor` instead: no
physics, no gameplay, only `update_in_editor` of each component.

`Scene::render` (see `render_passes` in `core/scene/scene.rs`) uploads the
scene's lights for the shaders, draws the weather maps (rain and snow
exposure), the cascaded shadow maps, the lamps' shadow cube maps, the
ambient occlusion from the camera's depth, then the scene into an HDR
target, which is tone-mapped (with bloom) to the screen. The editor draws
its UI on top.

## Multiplayer

`core::net` is always compiled in — the engine is the same whether a game
is offline, a client or a dedicated server. A game asks for `host` /
`connect` and then uses `request`/`take_events` on named channels for
world changes; the engine handles player broadcasts automatically. The
ferrite-server binary runs that same scene without a window or GPU
(see `headless.rs`).

## The MCP server

`ferrite-mcp` is a separate, authoring-time tool. It does not link the
engine or any GPU code; it reads and writes project files on disk the
way the editor would. It is meant for AI assistants to help with
authoring. See [mcp-server.md](mcp-server.md).

## Testing

```shell
cargo test --workspace
```

The tests cover what runs without a GPU: scene and project files, the
starter scene, the camera and projection math, the view frustum, terrain
chunk bounds, entity trees, finding scripts in a scene, the shapes' meshes,
the network protocol, the UI's text input and sliders, lights (settings,
placement, choosing the frame's lights, the moon, which lights get
shadows and which cube faces are drawn), the animation
graphs, scatter seeding, the weather, the editor's bridge handling
(property conversion, screenshots) and the whole MCP server, including the
bridge against a stand-in editor. Rendering and the editor's interactive
flows are checked by running them; there is no CI for those.
