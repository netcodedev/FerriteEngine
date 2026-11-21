# Graphics settings: plan

Status: plan (2026-11-21), nothing built yet.

## Why

Almost every value that decides how much a frame costs is a constant in
the code (view distance, LOD distances, shadow resolution, MSAA, ...). The
user wants (2026-11-21):

1. Settings for the **editor and the game**: tune them in the editor, and
   let a game's players choose them in an options menu.
2. **Presets** Low / Medium / High / Ultra. Today's values are **High**;
   Ultra goes beyond them.
3. **Project-specific settings**: a project can add settings of its own.
4. **Values chosen by setting inside assets**: e.g. the distance where a
   scatter graph's grass fades out, today a fixed number in the graph.
5. Everything in **one branch**, not in rounds.
6. On first start the project's **fixed preset**, no hardware detection.

## What exists today

| What | Where | Value |
|---|---|---|
| Terrain view distance | `StreamRadius::DEFAULT` (terrain/streaming.rs) | load 720 m, unload 848 m, full detail 300 m |
| Loaded-columns picture | `REACH` (terrain/columns.rs) | 12 columns = 1536 m around the camera |
| Terrain LOD | `LOD_DISTANCES` (terrain/chunk.rs) | 160 / 320 / 560 m |
| Scatter LOD | `LOD_DIST_*` (terrain/scatter/mod.rs, used in instances.rs) | 60 / 130 / 250 m |
| Detail scatter range | `ScatterRange::Detail { distance }`, `fade`, `fade_mode` per graph output | per graph |
| Sun shadows | `SHADOW_DISTANCE`, `SHADOW_CASCADES` (light/skylight.rs; `SHADOW_CASCADES` also in pbr/lighting.glsl) | 300 m, 4 cascades |
| Sun shadow map size | **scene file**, `SceneSettings.shadow_map_size` | 4096 |
| Cascade stagger | `ShadowSettings.stagger_second_cascade` (sky light) | off |
| Point light shadows | `SHADOW_RESOLUTION`, `MAX_SHADOWED_LIGHTS` (light/shadow.rs) | 512 px, 8 lights |
| MSAA | `window_samples` (gfx/vulkan/context.rs, gl_backend.rs) | highest of 8/4/2 the driver offers |
| SSAO | `AmbientOcclusionSettings` (scene look) + `SAMPLES` (ssao/mod.rs) | on, 16 samples |
| Bloom | `HdrSettings.bloom` (scene look), `BLOOM_LEVELS` | on, 6 levels |
| Clouds | `CLOUD_DOWNSCALE` (sky/sky.rs) | half resolution |
| Precipitation | `MAX_DROPS` / `MAX_FLAKES` (weather/precipitation.rs) | 14 000 / 9 000 |
| Anisotropic filtering | `SamplerDesc::anisotropic` exists in gfx | mostly 1× |
| VSync | `window::set_vsync`, editor stats panel, MCP `vsync` | on |
| Project file | `ProjectFile` (core/project.rs): `version`, `name`, `startup_scene`, `editor` | no graphics section |

## The idea

### Look versus cost

Two kinds of values must stay apart:

- **Look** (exposure, bloom intensity, AO radius and strength, cloud type,
  a scatter output's own fade): how something should look. Stays in the
  scene, sky light or asset, as today.
- **Cost** (resolution, distances, sample counts, map sizes): how much the
  machine may spend. Moves into the graphics settings.

Where both meet, the scene says *whether* (SSAO on), the settings say
*how well* (8 or 16 samples) and may switch it off (Low).
`shadow_map_size` is cost and leaves the scene file: still read from old
scenes (ignored, with a log line once), no longer written.

### One registry of settings

There is no built-in list. A **registry** holds setting definitions; the
engine registers its own at start, the project file adds more. The editor
panel, the player's file, MCP and the script API only work through the
registry, so a project's own setting shows up everywhere at once.

A definition:

```rust
pub struct SettingDef {
    pub key: String,          // "shadows", "demo.blood_decals"
    pub label: String,        // "Schatten"
    pub group: String,        // "Lighting": heading in menus
    pub kind: SettingKind,
    pub hidden: bool,         // project may hide engine settings it has no use for
    pub description: String,
}

pub enum SettingKind {
    /// Low / Medium / High / Ultra. Follows the preset.
    Level,
    /// On or off, a value per level of the preset.
    Toggle { per_preset: PerLevel<bool> },
    /// A number in a range, a value per level of the preset.
    Number { min: f32, max: f32, step: f32, unit: String, per_preset: PerLevel<f32> },
    /// One of named options, an option per level of the preset.
    Choice { options: Vec<String>, per_preset: PerLevel<String> },
}

pub struct PerLevel<T> { pub low: T, pub medium: T, pub high: T, pub ultra: T }
```

**Levels per group.** Most user-facing settings are of kind `Level`, one
per group ("Schatten: Hoch", "Vegetation: Mittel"), as in most games. The
**preset** sets every setting to its value for that level; changing one
setting afterwards makes the preset "Custom". Assets and engine values
never depend on the preset directly, only on a **named setting's level**,
so a player on Custom with low vegetation and high shadows gets exactly
that.

### Values chosen by a setting's level: `Param`

The same mechanism serves the engine's own knobs, project overrides and
assets:

```rust
pub enum Param<T> {
    Fixed(T),
    ByLevel { setting: String, values: PerLevel<T> },
}
impl<T: Copy> Param<T> {
    pub fn resolve(&self, settings: &Resolved) -> T;
}
```

In RON a fixed value is a plain value, so old files load unchanged:

```ron
distance: 60.0
distance: (setting: "vegetation", low: 30.0, medium: 45.0, high: 60.0, ultra: 90.0)
```

serde's `untagged` with RON is unreliable for this shape, so `Param` gets
a hand-written `Deserialize` (number/bool/string → `Fixed`, struct →
`ByLevel`), covered by round-trip tests.

**Engine knobs are `Param`s too.** Each built-in value (shadow map size,
cascade count, LOD factor, ...) is an engine default `Param` bound to its
group's level (table below). The project file may override any table. So
"what does Low mean" is data, not code, and a project tunes it without
touching the engine.

### Layers

1. **Engine:** built-in definitions and knob tables (High = today).
2. **Project** (`.fp`, new `graphics` section): default preset for the
   first start, extra settings, hidden settings, overridden knob tables.
3. **Player** (`graphics.ron` in the user's config folder,
   `$XDG_CONFIG_HOME/ferrite/<project name>/graphics.ron`, or the
   platform's equivalent): the chosen preset plus each setting changed
   by hand, by key. Unknown keys are kept (a newer or older game build may
   know them), invalid values fall back with a warning.

```ron
// project.fp
graphics: (
    default_preset: Medium,
    settings: [
        (key: "demo.blood_decals", label: "Blutspuren", group: "Effekte",
         kind: Toggle(per_preset: (low: false, medium: true, high: true, ultra: true))),
    ],
    hidden: ["effects.precipitation"],
    knobs: {
        "terrain.load_distance": (setting: "view", low: 400.0, medium: 560.0, high: 720.0, ultra: 1000.0),
    },
)
```

The editor and an exported game read the **same** player file, so the
editor shows what a player on this machine would get. The editor panel
also edits layer 2 (a separate tab "Projekt-Vorgaben").

The headless server (multiplayer) loads the registry but has nothing to
render; settings never reach it, see the rule below.

### Applying a change

`graphics::resolved()` returns an `Arc<Resolved>`: every setting's
current value plus every knob resolved, and a **generation** that goes up
on each change. Systems keep the generation they last applied and compare
once per frame. A change is applied in one of four ways:

| How | Settings |
|---|---|
| **Live** (read each frame) | LOD factors, shadow distance, cascade stagger, detail fade, vsync, FPS cap, bloom on/off, SSAO samples, precipitation amount |
| **Targets** (rebuild render targets, same path as a window resize) | MSAA, resolution scale, sun shadow map size, cascade count, point-light shadow size, cloud resolution |
| **Pipelines** (Vulkan: sample count is part of the pipeline; they are cached in ~/.cache/ferrite/vulkan) | MSAA |
| **Reload** (rebuild content) | view distance (`Streamer::set_radius`: queues columns now in reach, drops the rest past `unload`), detail scatter (rebuild detail cells of graphs that use a changed setting), anisotropy (re-create samplers) |

Changes in the menu apply at once; there is no "Apply" button, except
that the player's file is written when the menu closes (`graphics::save`).

### The rule: graphics settings change the look, not the world

Two players with different settings must have the same world: the same
colliders, the same trees to fell, the same plans in the world cache
(whose fingerprint does not know settings). So:

- A bound `Param` in a scatter graph only takes effect on **detail**
  outputs. Any path from a bound field to a **landscape** output resolves
  that field at **High**, whatever the settings say, and the node editor
  marks the node with a warning ("Landscape-Ausgabe: Grafikeinstellung
  wird ignoriert, nutzt Hoch").
- Detail outputs have no colliders, so they may vary. (To verify in step
  6: that no detail placement goes into the world cache on disk; a reload
  keeps loaded scatter in memory, which is fine as it is rebuilt on a
  settings change anyway.)
- Engine settings never touch generation: view distance changes what is
  *loaded*, not what is *generated* for a column.

## Settings (engine)

Levels: L / M / H / U. **H = today's value.**

### Anzeige (display)

| Key | Kind | L | M | H | U | Applied |
|---|---|---|---|---|---|---|
| `display.msaa` | Choice off/2×/4×/8× | off | 4× | 8× | 8× | targets + pipelines |
| `display.resolution_scale` | Number 50–100 % | 75 % | 100 % | 100 % | 100 % | targets |
| `display.vsync` | Toggle | on | on | on | on | live |
| `display.fps_cap` | Number 0 (none)–240 | 0 | 0 | 0 | 0 | live |
| `display.anisotropy` | Choice 1×/4×/8×/16× | 1× | 4× | 8× | 16× | reload samplers |

MSAA is capped by what the driver offers (today's `window_samples`
search); the menu greys out what is not available. Resolution scale draws
the HDR target smaller and scales it up in the composite pass (bilinear).
Note: anisotropy at H = 8× is the one change from today's look; it should
only sharpen ground textures at grazing angles.

### Sichtweite (`view`, level)

| Knob | L | M | H | U |
|---|---|---|---|---|
| `terrain.load_distance` (m) | 400 | 560 | 720 | 1350 |
| `terrain.detail_distance` (m) | 200 | 250 | 300 | 450 |
| `terrain.lod_factor` (× `LOD_DISTANCES`) | 0.6 | 0.8 | 1.0 | 1.5 |

`unload` stays `load + CHUNK_SIZE`. The loaded-columns picture's `REACH`
becomes derived from the largest `load` any level can ask for (Ultra
1350 m + 128 → 12 columns still fits; projects that raise Ultra beyond
that grow the picture). Ultra loads ~3.5× the columns of High; generation
is off-thread, but memory and the first load grow accordingly; measured
in step 9. More than ~1.5 km only once chunks are saved (see the chunk
persistence goal).

### Schatten (`shadows`, level)

| Knob | L | M | H | U |
|---|---|---|---|---|
| `shadows.map_size` | 1024 | 2048 | 4096 | 4096 |
| `shadows.distance` (m) | 150 | 220 | 300 | 450 |
| `shadows.cascades` | 2 | 3 | 4 | 4 |
| `shadows.stagger_second` | on | on | off | off |
| `shadows.point_resolution` | 256 | 512 | 512 | 1024 |
| `shadows.point_lights` | 2 | 4 | 8 | 8 |

`SHADOW_CASCADES` stays 4 as the array size (Rust and GLSL); a uniform
`shadowCascadeCount` tells the shader how many are in use. The stagger
moves from `ShadowSettings` (sky light) into the settings; old sky data
with the flag still loads.

### Vegetation (`vegetation`, level)

| Knob | L | M | H | U |
|---|---|---|---|---|
| `scatter.lod_factor` (× `LOD_DIST_*`) | 0.6 | 0.8 | 1.0 | 1.5 |
| `scatter.detail_distance_factor` | 0.5 | 0.75 | 1.0 | 1.4 |
| `scatter.detail_density` | 0.5 | 0.75 | 1.0 | 1.0 |

The two detail factors apply to **fixed** values in graphs, so graphs
that don't bind anything still get cheaper on Low. A **bound** value
(`ByLevel`) is taken as it is, factor not applied: the author has said
exactly what each level gets. `detail_density` thins detail points by
their own random number (the same thinning `FadeMode::Thin` uses), so
lower density never moves a blade, it only drops some.

### Effekte (`effects`, level)

| Knob | L | M | H | U |
|---|---|---|---|---|
| `effects.ssao_samples` (0 = off) | 0 | 8 | 16 | 16 |
| `effects.bloom` | off | on | on | on |
| `effects.cloud_downscale` | 4 | 2 | 2 | 1 |
| `effects.precipitation` (share of max drops) | 0.3 | 0.6 | 1.0 | 1.0 |

SSAO and bloom stay switchable in the scene's look; the settings can only
switch them *off* or lower them, never on where the scene has them off.

## Scatter graphs

Fields that become `Param`:

| Node | Field | Type |
|---|---|---|
| `SpawnMesh` | `range: Detail { distance }` | `Param<f32>` |
| `SpawnMesh` | `fade` | `Param<f32>` |
| `SpawnMesh` | `shadows` | `Param<bool>` |
| `SpawnMesh` | `wind` | `Param<f32>` |
| `SurfacePoints` | `spacing` | `Param<f32>` |
| `RandomPoints` | `per_hectare` | `Param<f32>` |

Others stay plain until a need shows up; adding one later is a local
change (field type, `edit.rs` property, a test). `fade_mode` stays plain.

Evaluation gets the `Resolved` settings next to the seed and cell, so it
stays a pure function: same graph, seed, cell **and settings** → same
placements. `ScatterGraph::settings_used()` lists the keys a graph's
bound fields name; the terrain rebuilds a graph's detail cells only when
one of those changes.

**Node editor / inspector** (`graph/edit.rs` `Property`, editor
`scatter/canvas.rs`): a bindable field gets a small link button. Bound,
it shows a setting picker (only `Level` settings) and one row per level;
the node's title shows e.g. "Gras, 45–90 m". Unbinding keeps the High
value. Undo/redo goes through the existing `GraphHistory`. The graphics
panel has a "Vorschau-Stufe" so the author sees each level in the
viewport without changing the player file.

**Why no value pins.** The graph's wires carry points, not numbers. Value
pins mean typed pins, number nodes and changes to evaluation and the
node editor: a feature of its own. `Param` covers "a value per level". A
"Graphics setting" node with value pins can come later if values need
maths ("distance × 1.5").

**MCP:** the scatter graph tools accept the `ByLevel` form wherever they
accept a number for these fields.

## Scripts and MCP

Scripts (Rust, see docs/scripting.md) get `ferrite::graphics`:

```rust
graphics::level("vegetation") -> Level
graphics::get_bool("demo.blood_decals") -> bool
graphics::get_f32("display.resolution_scale") -> f32
graphics::set("vegetation", Level::Low)        // marks the preset Custom
graphics::set_preset(Preset::Medium)
graphics::definitions() -> Vec<SettingDef>     // to build a menu
graphics::save()                               // write the player file
graphics::generation() -> u64                  // changed since?
```

and `Behaviour::on_graphics_changed(&mut self, ctx)`, called once after a
change, for scripts that act on a project setting (blood decals on/off).

A script parameter can be a `Param<f32>` too, so an entity's own values
(a torch's light range) can follow a level. The inspector shows it like
in the graph.

MCP: `graphics_get` (all definitions + current values + generation),
`graphics_set` (key/value or preset), `graphics_preview_level`. The
existing `vsync` switch maps onto `display.vsync`.

The demo gets an options menu (Esc → Optionen → Grafik), built from
`graphics::definitions()`, grouped by `group`, nothing hard-coded.

## Rules and how each is checked

| # | Rule | Check |
|---|---|---|
| R1 | At High, the engine looks and performs as before this branch | Four fixed editor views (as in render-backends) diffed per pixel against master; mean within the noise floor (≤ 2.2). Frame time on the fixed flight within 5 %. |
| R2 | Old scenes, sky data, scatter graphs and project files load unchanged | Load every scene/graph in the demo and the starter project; tests for `shadow_map_size` and `stagger_second_cascade` in old files. |
| R3 | `Param` round-trips through RON in both forms | Unit tests: fixed number, fixed bool, `ByLevel`, write → read → equal. |
| R4 | A project setting appears in editor panel, player file, MCP and script API without engine code | Test project with one Toggle; test reads it through all four. |
| R5 | Presets set every setting; changing one makes Custom; picking a preset again overrides by-hand values | Unit tests on the registry. |
| R6 | Graphics settings never change landscape outputs, colliders or the world cache | Test: evaluate a graph with a bound field feeding a landscape output at L and U → identical placements; the world cache fingerprint is unaffected by settings. |
| R7 | Detail density only removes points, never moves one | Test: points at density 0.5 ⊂ points at 1.0, same positions. |
| R8 | Each change applies without restart and without a lasting hitch | Switch every setting L↔U while flying (fly.py) with `FERRITE_SPIKES=1`; a one-off spike at the switch is fine, none after. Both renderers (GL and Vulkan). |
| R9 | Each preset is visibly what it says | Screenshots at eye level (not from above: grass vanishes in top-down views) per preset at three fixed places; detail count read first. |
| R10 | The player file survives unknown and invalid keys | Tests: unknown key kept on save; out-of-range value clamped with a warning. |

## Steps

1. **Registry and `Param`** (`core/graphics/`): `SettingDef`, `Level`,
   `Preset`, `PerLevel`, `Param` with its `Deserialize`, `Resolved` with
   generation, engine definitions and knob tables. Tests R3, R5, R10.
2. **Layers**: project file `graphics` section, player file read/write,
   first start with the project's preset. Tests R2 (project), R4 (file).
3. **Live knobs**: terrain and scatter LOD factors, shadow distance,
   stagger, vsync, FPS cap, bloom, SSAO samples, precipitation.
4. **Targets and pipelines**: MSAA (GL and Vulkan, greyed-out levels),
   resolution scale (HDR target + composite), sun shadow map size out of
   the scene file, cascade count uniform, point shadow size and count,
   cloud downscale.
5. **Reload**: `Streamer::set_radius`, horizon hole from the resolved
   load distance, `REACH` derived from the largest level; anisotropy.
6. **Scatter graphs**: `Param` fields, `settings_used`, landscape rule
   (R6, R7), detail factors, rebuild on change, node editor binding UI,
   MCP forms.
7. **Editor panel** "Grafik": preset, groups, this-machine vs project tab,
   preview level, frame time next to it; MCP `graphics_*`.
8. **Script API** + `on_graphics_changed` + `Param` script parameters;
   demo options menu; one demo setting of its own to prove R4 in a real
   project (e.g. "Gras unter Straßenrand" or whatever the demo needs).
9. **Verification**: R1, R8, R9 on both renderers; measure Ultra's
   memory and first-load time; write the as-built notes into this file.

## Open questions

- Values in the tables above are first guesses (except High). They get
  tuned in step 9 by measurement; the user may want to set some.
- The demo's own setting for R4: which one is useful?
