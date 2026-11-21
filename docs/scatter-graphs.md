# Scatter graphs: plan

Status: plan (2026-11-21), nothing built yet. Name: Scatter Graph (not
"PCG", which is Unreal's term).

## Why

The world looks good but empty (wishlist item 3, 2026-11-18): no grass, few
stones, no bushes. The wish is a system like Unreal's procedural foliage:
rules the user puts together themselves, filters on the points (slope,
height, ...), and adjustments on the output (random rotation, scale, ...).
First user: grass from `lowpoly_grass_link_in_description.glb`.

## What exists today

`terrain/scatter` already scatters models over the terrain:

- A terrain has **layers**: model (asset or file) + density (attempts per
  hectare, max 200) + tag. Saved in the scene file (`ScatterLayerData`).
- Per **chunk (128 m)** and layer, `scatter_spots` draws `attempts` random
  spots from a hash of (column, salt), so they are deterministic.
  `planting.rs` asks the terrain source on 2 worker threads:
  `surface_height` and `scatter_chance(tag, x, y, z)`. The demo's
  `scatter_chance` knows water, roads, towns, power-line corridors and the
  biome's vegetation.
- `ScatterInstances` draws them instanced per (variant, LOD), culled in
  cells; models get trimesh colliders (trees can be felled, `removed`).
- `far_scatter` draws impostors on the same spots beyond the loaded chunks.
- The PBR material already cuts out alpha < 0.5 (foliage cards).

What is missing for grass and a rule system:

1. **Scale.** Trees: ~20 per hectare. Grass: several per m² — 30 000+
   per chunk. Planting that over every loaded chunk, with colliders and
   impostors, is far too much. Grass is only needed within ~50–80 m.
2. **Rules.** The only rule is "chance from the game's source × density".
   There is no slope or height filter, no scale/tilt, no "no grass under
   rocks", no evenly spaced points (random spots clump and leave gaps).
3. **Look.** No wind, no double-sided drawing, no fade-out at the edge of
   the range, no colour match with the ground.

## The idea

A **scatter graph** is an asset (`*.scatter.ron`, next to `*.clip.ron`):
a small graph of nodes that turns a piece of ground into placements. The
terrain gets a list of graphs (as it has layers today). The graph is pure
data and its evaluation a pure function of (graph, seed, cell, terrain
source), so the result is the same every time and can be cached or saved
later (see chunk persistence: generation may get slower, runtime may not).

### Nodes (v1)

Points flow from top to bottom; every point carries position, normal,
slope, height, a density value 0..1 and a random seed.

**Sources** (make points)

- `Surface points` — evenly spread points on the ground of the cell:
  jittered grid (spacing in m, jitter 0..1); optional minimum distance
  (Poisson-like). Replaces the clumpy random spots for dense things.
- `Random points` — today's behaviour (attempts per hectare), for sparse
  things like trees, so existing worlds keep their tree spots.

**Filters** (drop points or lower their density)

- `Slope` — keep between min/max degrees (soft edge).
- `Height` — keep between min/max world height (soft edge).
- `World chance` — multiply by the game's `scatter_chance(tag)`: water,
  roads, towns, biomes stay the game's knowledge, written once.
- `Noise` — multiply by a noise mask (scale, threshold, contrast): patches
  and clearings instead of an even carpet.
- `Density cut` — keep a point if its density > its random number (the
  step that turns soft values into "there or not").
- `Keep away from` — drop points near the output of another graph or node
  (radius): no grass in rocks, bushes not inside tree trunks.

**Transforms** (change the points)

- `Rotate` — random yaw range; tilt with the ground normal (0 = upright,
  1 = fully along the slope); random extra tilt.
- `Scale` — random uniform range, optional extra height range; scale by
  density (thinner at the edge of a patch: smaller).
- `Offset` — move along the up axis (sink rocks into the ground).

**Output**

- `Spawn mesh` — model asset, variants with weights, **range** (near:
  only within N m of the camera; far: like today's trees, whole loaded
  world + impostors), collision on/off, cast shadows on/off, wind amount.

Graphs are evaluated off the main thread on the existing planting workers.

### Two ranges: detail and landscape

- **Landscape** outputs (trees, big rocks) work as today: per chunk, with
  colliders, felling and far impostors. The existing layers become graphs
  of `Random points → World chance → Density cut → Rotate → Spawn mesh`.
- **Detail** outputs (grass, flowers, pebbles) live in small cells
  (16 m × 16 m) in a ring around the camera, up to the output's range
  (default 60 m). Cells are planted as the camera comes close and
  dropped as it leaves; no colliders, no impostors. Past ~80 % of the range
  the instances shrink to nothing (no hard edge where grass pops).
  The ground's own texture carries the green further out.

### Drawing grass

- Double-sided (no back-face culling) for detail outputs, with the
  normal facing the camera side so both sides are lit.
- Alpha cut-out (exists), never alpha blending: the GLB's materials are
  `BLEND` and are read as cut-out.
- Wind: a vertex sway in the scatter vertex shader, stronger towards the
  top of the model (by local height), phase from world position; one
  global wind direction + strength (later from the weather).
- Ground tint: the base of the blade takes on the terrain colour under it
  (optional, per output), so grass matches dry or lush ground.
- Shadows: off by default for grass (cost), on for bushes and rocks.

**Structure** (because graphs branch, decided 2026-11-21)

- `Merge` — joins the points of several inputs.
- `Split` — sends points above / below a density or random threshold
  down two outputs (e.g. 80 % grass, 20 % flowers from the same points).
- A graph may have several `Spawn mesh` outputs.

### Editing: node editor (decided 2026-11-21)

A graph asset opens in its own editor tab (like the clip editor), built
in `ferrite-ui`:

- canvas with pan and zoom, grid;
- nodes as boxes with input pins on the left and output pins on the right,
  coloured by what flows (points; later maybe numbers/masks);
- drag from a pin to a pin to wire; a wire into an occupied input replaces
  the old one; only type-matching pins connect, no cycles;
- right-click (or Space) opens a searchable "add node" menu; Delete
  removes; box select, drag to move, copy/paste, undo/redo through the
  editor's existing undo;
- the selected node's properties in the inspector;
- each node shows its point count from the last evaluation (cell under
  the camera), so a filter that drops everything is seen at once.

Edits replant after the same short delay as today. Node positions are
saved in the asset (editor-only data).

Debug views (from the editor's debug menu), showing what is really
planted, never plan data:

- point density per graph as a colour on the ground;
- counts per output and cell, time per cell in the profiler.

## The grass model

`~/Downloads/lowpoly_grass_link_in_description.glb` (Sketchfab, 1.3 MB):

- 6 meshes: 3 flat cards (6–8 vertices) and 3 clumps (60–104 vertices),
  each ~1–2 m across in the file; 2 PNG textures.
- Materials `BLEND` + double-sided → read as cut-out (see above).
- The meshes lie in a Z-up frame and the file turns them with a node
  transform; the engine's model loader does not apply node transforms
  (`PreTransformVertices` is off), so the grass may come in lying flat.
  First step checks this; fix at import (bake node transforms for
  scatter models) rather than in the file.
- Licence **CC-BY-4.0**, author Atrikumar Das: needs a credit line in the
  demo (credits file / about screen).
- Scale: the clumps are ~1 m tall in the file, too tall for grass; the
  graph's `Scale` node (e.g. 0.3–0.6) handles that.

## Rules and how each is checked

| Rule | Check |
|---|---|
| Same world, same placements, every load | test: evaluate a graph twice and on two threads → identical |
| Existing tree/rock worlds keep their spots | test: old layer → converted graph gives today's `scatter_points_at` points |
| No grass on water, roads, in towns, under power lines | `World chance` uses `scatter_chance`; test on the demo source at known road/water points |
| Slope / height filters do what they say | unit tests with a flat and a 45° test source |
| `Keep away from` respects the radius | unit test |
| Detail cells only within range; dropped when leaving | test with a moving camera; instance count bounded |
| No frame spikes from planting | planting stays off the main thread; profiler: frame time while flying fast over grass |
| Looks right in the real demo | editor screenshots taken by me, not plotted data; the user flies around |

Budget to aim for (to measure, not promised): grass within 60 m at
~2 instances/m² is ~22 000 instances in view, a few draw calls per cell
and variant; frame cost of grass < 2 ms on the user's machine.

## Steps

1. **Import the grass.** Done 2026-11-21, differently than planned: the
   file is fixed, not the engine. Baking node rotations alone would not
   have been enough: the engine finds variants by LOD names, and the
   Sketchfab file has none, so all six meshes would have become one
   variant, stacked. `tools/grass_model.py` (demo, Blender headless)
   bakes the node chain, centres each mesh on its base and writes
   `assets/models/grass/grass.glb` with six variants `grass_card_1..3`,
   `grass_clump_1..3` (each `<name>_LOD0`). Scatter pipelines already
   draw both sides (`Cull::None`) and cut out alpha < 0.5; the sidecar
   marks the materials as foliage (no collision). Credit in the demo's
   `CREDITS.md`. Checked in a scratch project's editor (asset preview
   and as a terrain layer): upright, blades cut out cleanly. Later
   (engine): name-free multi-mesh packs as variants, node rotations.
2. **Graph data + evaluation** (engine). Done 2026-11-21:
   `terrain/scatter/graph/` — `ScatterGraph` (nodes with stable ids,
   wires pin to pin, `problems()` finds broken wires, doubly wired
   inputs and cycles), `evaluate(graph, ground, rect)`, and the asset
   kind `ScatterGraph` for `*.scatter.ron` files (imported into
   `assets/scatter/`). Sources lay points on world-wide structures (a
   grid; the columns' `scatter_spots`), so cutting the ground into cells
   changes nothing and `Keep away from` looks past a cell's edge by its
   radius. Points whose density falls to 0 are dropped. `Ground` is what
   a graph asks of the world (height, chance); every `TerrainSource` is
   one. The ground's normal is only sampled where a node needs it
   (`Slope`, `Rotate` with `align`). Tests: 13 in `graph/tests.rs` plus
   the import test, covering the rules table. Left out for now: the
   minimum distance of `Surface points`, and `Keep away from` another
   graph (it works within one graph).
3. **Layers → graphs**: terrain holds graphs; old scene layers load as
   generated graphs (no change to existing worlds); far scatter reads
   landscape outputs as before.

   Design (2026-11-21):
   - **No new scene field.** A scatter layer's asset may now be a
     scatter graph instead of a model; the scene stays
     `scatter: [(asset: ..., density: ...)]`. A model layer becomes
     `ScatterGraph::from_layer` inside (density and tag as before); a
     graph layer loads its file and the models of its outputs. The
     inspector shows density and tag only for model layers; the scatter
     asset picker offers graphs too.
   - **One spec per landscape output.** `ScatterSpec` carries the graph,
     the output node and a salt (a model layer keeps its old salt, so
     felled trees and spots stay; a graph layer's outputs get the layer
     salt mixed with the node id). Detail outputs plant nothing until
     step 4.
   - **Near.** The planting workers evaluate the graph on the chunk's
     column and keep the output's placements inside the chunk's height
     band (as `scatter_points_at` did). Instances draw the full
     placement (tilt, scale); colliders take position and tilt, not the
     scale (noted limit: scaled landscape models collide at size 1).
   - **Far.** `far_scatter` evaluates each landscape output per column
     on a `FarGround`: heights from the horizon grid, chance from
     `far_scatter_chance`, thinned with distance inside `chance` (before
     the costly lookup, as now), and once more on the output for graphs
     without a `World chance`. Impostors take yaw and the height scale.
   - **As built** (2026-11-21): as designed. Graph node ids are written
     as plain numbers (`id: 0`, `node: 2`), found when writing a graph by
     hand. Checked: the old-vs-new tests pass bit for bit; in the demo
     editor the culling counts at the start view are exactly those of
     the editor before the change (scatter cells 989 of 3622 drawn); a
     hand-written graph layer (random points, slope, tilt, 2–4x scale)
     planted leaning, scaled grass in a scratch project.
   - **Checks.** A test keeps the old far placement loop as a reference
     and compares it with the new one bit for bit for a converted layer;
     the planting job is compared with `scatter_points_at`; the existing
     far, felling and planting tests keep passing; then a look at the
     demo in the editor (same trees as before).
4. **Detail range**: camera ring of 16 m cells, planting on the workers,
   shrink-out at the edge, no colliders/impostors.

   Design (2026-11-21):
   - **Cells of 32 m, not 16.** Every cell is its own set of draws (one
     per variant and pass); at 60 m range 32 m cells are ~16 instead of
     ~50, a third of the draw calls. Planting a 32 m cell is ~2000
     points for grass at 0.7 m spacing, fine on a worker.
   - **Which cells.** Per detail output, every cell whose square comes
     within its `distance` of the camera (xz). A cell is dropped only
     when it is a cell's width further out, so walking along an edge
     does not plant and drop the same cell again and again.
   - **Where they live.** A cell's placements are split by height among
     the loaded chunks of its column, as a `ScatterInstances` component
     on each (marked with the cell, no colliders), so every pass that
     draws the landscape models draws them too and they go with their
     chunk. The landscape sync leaves components with a cell alone.
   - **Planting.** The same workers: a cell job evaluates the graph on
     the cell's square; the result is kept while the cell is wanted, so
     the column's chunks loading later take theirs from it. At most a
     few cells are added per frame.
   - **Shrink-out.** The instance vertex shader shrinks a detail instance
     about its base from 80 % of the range to nothing at the range (a
     uniform with the eye and the range; 0 for landscape instances), in
     every pass, so shadows and depth match.
   - **Shadows.** An output with `shadows: false` is left out of the
     shadow maps (still in the depth prepass); now for landscape
     outputs too.
   - **As built** (2026-11-21): as designed; the terrain's inspector
     shows "detail: N models in M cells". In the demo (scene
     `grass_check`, a first `Meadow` graph: 0.6 m grid, noise patches,
     slope ≤ 30°, world chance "grass", scale 0.35–0.7) a ring of 60 m
     holds ~23 500 tufts in 22 cells; frame 9.4 ms on average (GPU 7.1
     ms), from ~7.4 ms without. One cell of that graph takes ~20 ms to
     work out on the demo's world (release build). Seen from above the
     grass hardly shows: it is much darker than the ground (step 5).
   - **Checks.** Tests: the wanted cells for an eye and range, the drop
     margin, a cell's placements split over chunks lose and double
     nothing. Then grass in the demo editor: density, edge, frame time.
5. **Grass drawing**: wind sway, double-sided, ground tint, shadow switch.

   Design (2026-11-21), after looking at the grass in the demo:
   - **Double-sided** is already there (scatter pipelines draw both
     sides; `shadePbr` turns a back face's normal around). The shadow
     switch came with step 4.
   - **Why the grass is dark:** the texture's blades are dark olive
     (sRGB ~72, 81, 42), and a card's normal points sideways, so a high
     sun barely lights it. Two settings per `Spawn mesh`:
     `normals_up` (0..1) bends the shading normal towards the sky, so
     the blades are lit like the ground they stand on; `tint` (0..1)
     takes on the ground's colour.
   - **Tint.** `Ground::tint(x, z)` (default white) comes from the new
     `TerrainSource::scatter_tint`; the demo answers with the same biome
     tint its terrain shader puts on the grass texture, so the tufts turn
     dry or lush with the ground under them. Worked out per placement
     (only for outputs with `tint` > 0) and carried in the instance's
     matrix: its bottom row is always (0, 0, 0, 1), so it holds the tint
     (minus one) and the vertex shader puts the row back. No new vertex
     layout. The shader multiplies the albedo (`pbrTint`, a global
     `shadePbr` reads, white unless set).
   - **Wind.** One wind for the world for now (direction and strength;
     later from the weather), and the time, as uniforms; per draw the
     output's `wind` (0..1). A vertex moves along the wind by the square
     of its height in the model, swaying with two sines and slow gusts
     that roll across the ground. Same in every pass.
   - **As built** (2026-11-21): as designed, plus one fix found in the
     editor: the wind's time is set once a frame (`set_wind_clock`).
     Read per draw, it moved the blades a little between the depth
     prepass and the shading pass, which then missed most blade pixels,
     and the ambient occlusion drew them as black shapes. The demo's
     `scatter_tint` is its terrain shader's `grassTint`. With the meadow
     at `tint` 1, `normals_up` 0.6, `wind` 1 the tufts take on the
     ground's colour; frame 9.2 ms (GPU 7.3 ms); two frames a second
     apart differ in ~2 % of the grass pixels, not on bare ground.
   - **Checks.** Tests for the tint packing (matrix round trip, landscape
     instances untouched) and that `tint` 0 asks the ground nothing; then
     a look at eye level in the demo, and the frame time.
6. **Node editor**: graph asset in the asset browser, opens in its own
   tab (canvas, pins, wires, add menu, undo), properties in the
   inspector, point counts on the nodes, debug views. MCP tools to
   create/edit graphs (add node, connect, set property), so I can build
   and test graphs too. The biggest step; split into canvas + display
   first, then editing.
7. **Demo**: grass graph (meadows: lush biomes, gentle slopes, patches by
   noise, none on roads/water/towns), then stones and bushes with the same
   system when the user has models for them.

Each step ends with tests passing and, from step 4 on, a look in the real
editor.

## Decisions (the user's, 2026-11-21)

1. Name: **Scatter Graph** (assets `*.scatter.ron`).
2. Editing: a **real node editor** with wires from the start, not a list.
3. Grass is **looks only**: walk-through, no colliders, no ids per
   instance (detail outputs cannot be removed one by one).
