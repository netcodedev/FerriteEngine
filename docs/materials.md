# Materials: plan

Status: built (2026-11-21) on branch `feat/materials` (engine and demo), see "As built" at the end. Names: **Material** (the
asset a model's parts are drawn with) and **Material Graph** (the node
logic behind it). Not "Material Instance" (Unreal's term).

## Why

From the backlog of 2026-11-21: models use materials instead of raw
textures; materials are created and assigned automatically on import; users
build their own materials in a graph editor, with a preview on different
shapes. The user decided (2026-11-21) that **vertex offset and wind per
mesh part** belong in this round (the backlog's "wind per mesh part": for
trees only the leaves sway), and that **terrain materials come later**.

## What exists today

- **One fixed shading for every model.** `renderer/pbr/material.glsl` has
  five samplers (`baseColorMap`, `normalMap`, `roughnessMap`,
  `metallicMap`, `occlusionMap`) with `has*Map` flags. `shadePbr` samples
  them, cuts out alpha < 0.5, and calls `shadeSurface` (`lighting.glsl`).
  Special cases are globals set before the call: `pbrTint` (scatter
  instance tint) and `pbrNormalUp` (grass lit like the ground).
- **Model programs.** `model/model.rs` has two static programs: colour
  (`vertex.glsl` + `pbr::fragment_source(fragment.glsl)`) and depth. The
  scatter system has its own pair (`scatter_instanced_vertex.glsl` /
  `_fragment.glsl`), plus the impostor bake (`impostor_bake_*.glsl`),
  which repeats the colour map's cut-out.
- **Binding.** `pbr::bind_material(shader, textures)` binds a
  `HashMap<TextureType, Texture>` per assimp material index, kept in
  `Model::textures`. Callers fill it with `set_material_texture`,
  `set_material_color`, `set_material_map_file`: the model loader, the
  model component (asset overrides), scatter models, the rig editor,
  `shape_component` and `vegetation/tree_component` (generated textures),
  `generated.rs` (plain colours for houses, the pickup).
- **Settings on the model asset.** `MaterialSettings` (`assets/meta.rs`)
  per material name: optional texture assets per slot and `foliage`.
  `material_maps.rs` finds maps by file name; the importer uses it too.
- **Wind** exists only for scatter instances: `SpawnMesh::wind` (a
  `Param<f32>`, can be bound to a graphics level) becomes `windAmount`;
  `scatter_instanced_vertex.glsl` sways the **whole model** along the wind
  by the square of the height over its base, with two sines and gusts.
  `wind` (direction, strength, clock) is set per frame (`instances.rs`).
- **Meshes have no tangents and no vertex colours.** Normal maps use
  screen-space derivatives (`perturbNormal`). Vertex layout: position,
  normal, uv, bone ids, weights (instanced: position, normal, uv, instance
  matrix in 3..6).
- **Node editor.** `editor/src/editor/scatter/canvas.rs` (canvas, pins,
  wires) with its own inspector, undo and MCP tools; the animation editor
  has another canvas (`editor/anim/canvas.rs`).
- **Shaders** are GL GLSL; `gfx::glsl` rewrites them for Vulkan, SPIR-V and
  pipelines are cached in `~/.cache/ferrite/vulkan`. Exports have GL only
  (see the open "exports lack Vulkan" item).

## The idea

### Material Graph and Material

A **Material Graph** (`*.matgraph.ron`) is a graph of nodes with one
output node. It declares **parameters** (name, type, default): texture,
colour, float, vec2, bool. A graph compiles to GLSL once per structure.

A **Material** (`*.material.ron`) names a graph and sets its parameters,
plus render state:

```ron
(
    graph: Builtin(StandardPbr),          // or Asset(<id>)
    params: {
        "base_color_map": Texture(Some("<asset id>")),
        "base_color": Color((1.0, 1.0, 1.0, 1.0)),
        "roughness": Float(0.8),
        "wind": Float(1.0),
    },
    alpha: Cutout(0.5),                   // Opaque | Cutout(threshold)
    two_sided: true,
    foliage: true,                        // left out of collision shapes
    shadows: true,
)
```

Many materials share one graph and differ only in parameters: parameters
are uniforms and textures, so a new material costs no compilation.
Parameter values may be a `Param` bound to a graphics level (as in scatter
graphs), e.g. wind strength or a detail-normal switch per `vegetation`
level.

Materials also exist **in memory only**, without an asset: generated
models (houses, the pickup, procedural trees, shapes) build them in code
(`Material::standard().with_color(..)`, `.with_map(..)`). This replaces
`set_material_color` / `set_material_texture`.

### Builtin graph: Standard PBR

Shipped with the engine, not a file the user can break, but openable in
the editor (read only; "duplicate to edit"). Its parameters cover today's
behaviour exactly: the five maps, base colour factor, roughness / metallic
factors (glTF), normal strength, UV scale/offset, emissive colour, and the
wind parameters (see below). Drawn with Standard PBR and today's textures,
every model looks **pixel-identical** to today (rule R1).

### Shader generation

`shadePbr` splits in two:

```glsl
struct Surface {              // what a material produces per fragment
    vec3 baseColor; float opacity;
    vec3 normal;              // world space, after normal mapping
    float roughness; float metallic; float occlusion;
    vec3 emissive;
};
void evaluateMaterial(inout Surface s, MaterialInputs i);   // generated
vec4 shadeSurface(...);                                       // as today
```

and the vertex side gets the same:

```glsl
vec3 materialWorldOffset(VertexInputs v);                     // generated
```

The code generator walks the graph from the output node: nodes reaching
the fragment pins go into `evaluateMaterial`, nodes reaching the
**World Offset** pin go into `materialWorldOffset`; a node used by both is
emitted in both. Nodes that only work per fragment (screen-space
derivatives, `gl_FrontFacing`) are refused on the vertex side with an
error on the node; texture samples on the vertex side use `textureLod`.

Per graph the engine builds a **program family**, keyed by (graph hash,
pipeline): pipelines are *mesh* (skinned `vertex.glsl`), *instanced*
(scatter), *depth/shadow* for both, *impostor bake*. The depth and shadow
variants keep only what opacity (cut-out) and world offset need, so a
swaying leaf casts a swaying shadow and the depth prepass matches the
colour pass (the `invariant gl_Position` requirement stays). Programs are
built lazily on first use and cached in memory; Vulkan's existing SPIR-V
and pipeline caches cover the rest. Shared code (`lighting.glsl`,
weather, snow) stays as is; `applyWeather` runs after
`evaluateMaterial`.

Uniforms: parameters are named `mp_<name>`; samplers take texture units
from 0 upwards (lighting's units stay where they are). A graph may use at
most 12 texture parameters (checked in the editor).

### Nodes (v1)

| Group | Nodes |
| --- | --- |
| Inputs | Parameter, Constant (float / vec2 / vec3 / colour), UV (set 0), World Position, Object Position, Normal (world / object), View Direction, Time, Instance Tint, Height Over Pivot, Vertex Colour |
| Textures | Sample Texture (colour or data map, UV in), Normal Map (sample + strength → normal) |
| Maths | Add, Subtract, Multiply, Divide, Lerp, Clamp, Saturate, Power, One Minus, Abs, Min, Max, Dot, Normalize, Split, Combine, Remap, Step, Smoothstep |
| Effects | Fresnel, Noise (value, 2D/3D), Bend Normal Up (today's `normalUp`) |
| Settings | Graphics Level (picks one of four inputs by a level setting's value) |
| Wind | Wind Sway (see below) |
| Output | Material Output: Base Colour, Opacity, Normal, Roughness, Metallic, Occlusion, Emissive, **World Offset** |

Types: float, vec2, vec3, vec4; float widens to vectors, as in GLSL. Wires
of the wrong type are refused in the editor (red pin) and in the loader
(error with node id).

### Wind per mesh part

Wind moves from the scatter vertex shader into the material. **Wind Sway**
is a node that outputs a world offset, the same motion as today (along
the wind, by height over pivot squared, two sines plus gusts, phase from
the pivot's position) multiplied by:

- **amount** (pin): how much this part sways; usually a parameter.
- **mask** (pin, 0..1): which vertices sway. Typical sources: a constant
  (the whole material: leaves 1, bark 0.1), Vertex Colour (a channel
  painted in Blender; tree packs often ship wind weights there), a texture
  channel, Height Over Pivot (today's behaviour).
- **flutter** (pin): small fast motion along the normal for leaves,
  multiplied by its own mask.

"Only the leaves sway" is then: leaves material wind 1, bark material
wind 0 or a little, both with the same pivot motion, so leaves stay
attached where the crown bends little. Standard PBR exposes `wind`
(default 0), `wind_flutter` (default 0) and `wind_mask_source` (constant /
vertex colour R / height).

The world's wind (`wind` uniform: direction, strength, clock) becomes part
of the shared lighting state set for **every** model program, not just
scatter instances, so models placed as entities (a tree in a scene, a
flag) sway too. The pivot is the model's origin (entity) or the instance's
(scatter). `SpawnMesh::wind` stays as a per-output multiplier (0 turns
wind off for that output, a graphics level may lower it); the material
decides which parts sway.

**Vertex colours** are added to the mesh layout (one `vec4`, location 5
for meshes, 7 for instanced; default white when the file has none) since
they are the common way to paint wind weights. Cost: 16 bytes per vertex;
measured in step 2 (rule R7).

### Import and migration

- On import, each material in the file becomes a Material asset next to
  the model (`<model>/<material>.material.ron`, under the same folder as
  the model's textures), graph Standard PBR, parameters from today's
  sources: maps found by `material_maps` (and embedded ones), glTF factors
  (`GltfFactors`), `foliage` as today (name heuristics), two-sided and
  cut-out for foliage.
- The model asset's `MaterialSettings` become a slot list:
  `materials: [(name: "Bark_Mat", material: <asset id>)]`. Matching by
  name on reimport stays; a material the user changed is not overwritten
  (only new slots get new materials).
- **Migration**: one pass over the demo's 130 model assets turns each
  `MaterialSettings` into a Material asset (overrides → parameters). No
  compatibility layer: the demo is the only project (as with scatter
  layers). The old fields are removed.
- Embedded textures (glb) get extracted to texture assets at import so a
  material can refer to them by id.

### Editor

- **Shared node canvas.** The scatter canvas becomes a generic widget
  (`editor/node_canvas`) with node kinds, pin types and wire checks
  supplied by the user of it; scatter graphs move onto it first, unchanged
  in behaviour, then material graphs use it.
- **Material Graph workspace**: canvas, node inspector, parameter list
  (add, rename, type, default), compile errors on the node they belong to,
  undo. Live: every edit recompiles the preview program (debounced).
- **Preview** (beside the canvas): sphere, cube, plane, cylinder, or any
  model asset; lighting from `LightingState::preview` with a turnable sun,
  a fly camera (as in the scatter preview), wind on/off and strength to
  see world offset, a floor for shadows.
- **Material inspector** (selecting a `.material.ron`): graph picker,
  parameter fields (texture slots take dropped texture assets), render
  state, the same preview. "Open graph" jumps to the workspace.
- **Model asset inspector**: per slot a material field (drop a material
  asset, or "new material from this").
- **MCP**: `editor_material_read` / `editor_material_edit` (material
  parameters, render state) and `editor_material_graph_read` /
  `editor_material_graph_edit` (nodes, wires, parameters), plus
  `editor_material_preview_screenshot`.
- **Scripts**: `material.set_param(name, value)` on a model component
  (per-entity override, in memory, not saved), for e.g. a blinking light.

### Not in this round

Terrain materials (decided), blended transparency, refraction, per-entity
overrides saved in scenes, custom lighting models, tessellation/displacement,
material LOD (simpler graph at distance), subgraphs/functions.

## Rules and how each is checked

| # | Rule | Check |
| --- | --- | --- |
| R1 | Standard PBR with today's maps looks as today | Four fixed editor views (GL and Vulkan) diffed per pixel against master, noise floor as for the render backends (mean ≤ 2.2) |
| R2 | Import creates and assigns materials automatically | Test: import a glb and an FBX with loose maps → material assets exist, slots point to them, maps as `material_maps` finds them |
| R3 | Reimport keeps the user's material edits | Test: change a parameter, reimport → unchanged; a new material in the file → new asset |
| R4 | The demo's models all migrate | Migration pass over the demo; every model asset loads, no `MaterialSettings` left; R1 views of the demo unchanged |
| R5 | A graph compiles on GL and Vulkan | Unit tests generate GLSL for every node kind and for the builtin graph; the dumped-programs test compiles them with glslc |
| R6 | Only the chosen parts sway | Test scene: tree with leaves wind 1, bark 0 → two screenshots at different wind clocks; bark pixels identical, leaf pixels differ |
| R7 | No runtime cost for Standard PBR | Same overview view as in the render-backends measurements: scene render CPU and GPU within 5 % of master; vertex memory reported |
| R8 | Shadows and depth follow world offset | Shadow of a swaying leaf moves with it (screenshot pair); no prepass holes (LEQUAL test unchanged, checked in the debug depth view) |
| R9 | Bad graphs fail loudly, not black | Type errors and missing output refused with a message on the node; the draw falls back to a magenta Standard PBR, logged once |
| R10 | Grass and trees keep their look and motion | Meadow and trees at eye level (not from above) before/after; wind visually as before with mask = height |

## Steps

1. **Material type, Standard PBR fixed in code.** `Material` (graph ref,
   params, render state), in-memory constructors; `Model` holds a material
   per slot instead of texture maps; all callers moved (loader, model
   component, scatter models, rig editor, shapes, procedural trees,
   generated models). `shadePbr` split into `evaluateMaterial` (hand
   written Standard PBR) + `shadeSurface`. Check R1.
2. **Vertex colours, wind in the material.** Mesh layout with vertex
   colour; world wind set for all model programs; Wind Sway as GLSL in
   the hand-written Standard PBR, scatter shader's wind removed. Check R7,
   R10.
3. **Material asset and import.** `.material.ron` load/save/reload,
   asset database kind, import creates materials, migration of the demo.
   Check R2, R3, R4.
4. **Graph data and code generation.** Graph model, node kinds, type
   checks, GLSL generator for fragment and vertex, program families and
   cache; Standard PBR rebuilt as a builtin graph and checked to generate
   the same picture as step 1. Check R5, R8, R9, R1 again.
5. **Shared node canvas.** Scatter editor moved onto it, no change in
   behaviour (its tests and a look at the editor).
6. **Material editor.** Graph workspace, material inspector, previews,
   model asset slots, MCP tools, script API.
7. **Demo.** Tree materials: leaves sway, bark (almost) still; grass
   material with Bend Normal Up and tint instead of the scatter special
   cases. Check R6, R10 at eye level.

Each step builds and passes the tests before the next; R1 is rechecked
after steps 1, 3 and 4.

## Decisions (the user's, 2026-11-21)

- A Material System with a graph editor, materials automatic on import,
  preview on several shapes (backlog).
- Terrain materials later, not this round.
- Vertex offset and wind per mesh part in this round.
- Names: Material and Material Graph.

## Open questions

- File layout of imported materials: next to the model (planned) or one
  `materials/` folder per project?
- Should parameters of a graph be groupable (sections in the inspector)
  from the start?

## As built (2026-11-21)

Everything in the steps above is built; where it differs from the plan, it
says so here.

### Engine (`core/material/`)

- `graph.rs` (data, node kinds and pins), `codegen.rs` (graph → GLSL, type
  widths, checks), `standard.rs` (Standard PBR), `programs.rs` (templates
  and program cache by graph key, variant and pipeline), `mod.rs`
  (`CompiledGraph`, `Material`, binding), `asset.rs` (material files,
  graph assets, texture assets, import, migration, `create`), `edit.rs`
  (properties and tool ops shared by the editor's panels and MCP),
  `preview.rs` (preview shapes), `wind.rs` (the world's wind and its
  clock, set once per scene frame).
- GLSL pieces: `inputs.glsl` (`MaterialInputs`, noise), `vertex_prelude.glsl`
  (varyings, `materialWind`), `fragment_prelude.glsl` (varyings,
  `materialNormalMap`), `surface.glsl`, and `renderer/pbr/material.glsl`
  (`shadeMaterial`). The model, scatter and impostor-bake shaders are now
  templates the material's code goes in front of.
- Cut-out is a program variant (no `discard` in opaque materials); the
  depth-only programs of opaque materials have an empty fragment stage.
- Textures: `Texture::share` handles do not own. A model keeps every map
  `init` found (`Model::map_textures`); materials hold handles. Replacing
  a material (with a material asset's) once freed the textures other
  materials drew with (everything asset-based went black) — fixed that way.
- `Model::file_materials` keeps what the loader made of the file; material
  assets are built on top of it (`material::asset::apply_to_model`), so a
  texture parameter set to `Auto` is what the loader found.
- Binding sets only values that differ from what the program holds
  (`PROGRAM_VALUES`); samplers stay on their units per program; materials
  cache their programs. Without that the render CPU time was 13 % above
  master (every house has material instances of its own).
- **Wind node, changed from the plan:** a fifth input "Höhe" (reference
  height, default 1 m): the bend is `0.35 · H · (h/H)² · sway`. With 1 m it
  is the old grass formula bit for bit; trees need their height there, or
  a 20 m tree would bend by tens of metres. Standard PBR's parameter:
  `wind_height`.
- Instance tint and "normals up" of scatter outputs stay draw settings
  (`pbrTint`, `pbrNormalUp`), applied to every material; the graph has
  "Instanz-Tönung" and "Normale nach oben" nodes for its own use.
- `FERRITE_FIXED_CLOCK=<seconds>` holds the weather/wind clock still
  (comparing pictures of two builds).
- Vertex colours: one packed `u32` per vertex (4 bytes, not the planned
  16), `unpackUnorm4x8` in the shaders.

### Assets and migration

- `AssetKind::Material` and `AssetKind::MaterialGraph`; `MaterialSettings`
  of a model slot is now `name` + `material`; the old map overrides and
  `foliage` are read (never written) so the migration can move them.
- `ensure_all_model_materials` runs when the editor opens a project and on
  Refresh: every slot without a material gets
  `<model>_<material>.material.ron` beside the model. Materials of models a
  scatter graph plants with wind get `wind: 1` (they swayed as a whole
  before). The demo has 387 of them.
- Import makes them too; re-importing keeps existing ones.
- New materials and graphs: the asset browser's "Material" / "Graph"
  buttons and the MCP tool `editor_material_create` (into
  `assets/materials/`; a new graph starts as a copy of Standard PBR).

### Editor

- `graph_canvas.rs`: the scatter canvas made generic (`CanvasDoc`,
  `CanvasHost`); nodes grow with their pin rows. Scatter graphs use it
  unchanged.
- `material/`: `MaterialEdit` (document, undo, file following), canvas
  binding, inspector (material settings and parameters with texture pick
  buttons and drop targets; a graph's selected node, or its parameters to
  add, rename, retype, remove), preview (sphere/cube/plane/cylinder, turn,
  zoom, wind on/off; `Auto` textures come from the first model that uses
  the material). Model asset inspector: one "Material" row per slot (pick,
  drop, edit).
- MCP: `editor_material_read`, `editor_material_edit` (`set` for a
  material, `ops` for a graph, `save`), `editor_material_preview` (open on
  a shape; take `viewport_screenshot` with `source: window`),
  `editor_material_create`.
- Scripts: `ModelComponent::set_material_param(material, param, value)`
  and `clear_material_params()` (per entity, in memory, kept over material
  reloads).

### Demo

- `scripts/building_parts.rs` copies the source's first textured material
  (`material_textures` is gone).
- Tree leaves (`Trees_leav`, `Trees_lod_last`, `Acer…Cluster_Mat`, Acer
  billboards): `wind 0.03`, `wind_flutter 2`, `wind_height 12`; trunks
  (`Trees_Material_001`, `Acer…Bark_Mat`): wind 0. `Trees.scatter.ron`'s
  outputs have `wind: 1.0`. Only the crowns sway.

### Checks (2026-11-21, Vulkan, demo copy, fixed clock)

| Rule | Result |
| --- | --- |
| R1 | Five views against a second master run: mean 0.03–0.11 where nothing sways, grass 0.44 once its wind was migrated. (Two master runs differ by up to 9: the first master run is no reference; compare against a second.) |
| R2, R3 | Unit tests (import makes a material per slot; legacy slots load and migrate; counts). |
| R4 | 132 model assets migrated, all load; scene unchanged (R1). |
| R5 | `generated_programs_compile_for_gl_and_vulkan` (ignored test, about three minutes): every node kind and Standard PBR, all pipelines and variants, glslangValidator and glslc. |
| R6, R8 | Two frames 0.7 s apart at eye level: only crowns, grass and their shadows change; trunks, poles, houses identical. |
| R7 | Overview view, vsync off, median of 8: render CPU 6.13 ms (master 6.41), GPU 4.81 ms (master 4.64, +3.7 %). |
| R9 | A graph reading a removed parameter: preview magenta, node framed red, footer says why; a wire that would break a graph is refused. |
| R10 | Grass sways as before (R1 view 2); trees now sway at the crown only, on purpose. |

### Open

- Terrain materials (decided: later).
- The preview can only fill `Auto` textures from a model that uses the
  material.
