# World cache: plan

Status: built (2026-11-20); see "As built" at the end.

## Why

Starting the editor on the demo takes ~15 s until terrain shows, and every
script reload pays ~12 s again, even for scripts that have nothing to do
with the world (a vehicle tweak). Measured: the demo's world planning
(hydrology, roads, power grid; one "world plan" thread) takes ~10 s, and
every chunk worker, the horizon, planting and far scatter wait for it.
Chunk meshing after that is a few seconds at most; the far scatter ~0.5 s.

## What it does

1. **Fingerprint of a world.** `terrain::WorldKey`: generator name, seed,
   settings (RON text), the source hash of the generator's scripts, the
   engine's build stamp and the terrain mesher. The source hash covers the
   module that registers the generator and every script module it names
   (`crate::<module>`), transitively — for the demo `landscape` and
   `power`, not `vehicle` or `building`. `crates/scripts/build.rs` computes
   it and registers it with the generator.

2. **Cache directory per world.** `<project>/.ferrite-cache/world/<fingerprint>/`
   (ignored by git). The engine hands the directory to the generator
   (`terrain::world_cache_dir()` while `create` runs); what a generator
   keeps there is its own business. Old fingerprints' directories are
   removed, keeping the newest few.

3. **The demo keeps its plans there.** Hydrology, roads and the power
   grid get serde derives; `start_planning` loads them when the file is
   there and of this fingerprint, plans and saves them otherwise.

4. **A reload with the same fingerprint keeps the world.** When the
   scripts are reloaded and the terrain's fingerprint did not change, the
   loaded chunks (engine data: meshes, colliders, scatter) stay; only the
   source is made again from the new library (fast: its plans load from
   the cache) and streaming goes on from the loaded columns.

5. **Regenerate.** A "Regenerate world" button on the Terrain component
   deletes this world's cache directory and builds the world again: for
   changes the fingerprint cannot see (a generator reading a data file).

## Not now

- Chunk meshes on disk (hundreds of MB for the loaded radius; to measure
  once the plans are cached: if chunk meshing then dominates, cache the
  LOD meshes per (coord, detail) as raw vertex/index bytes and rebuild
  colliders from LOD0).
- A sample check that re-plans in the background and compares: the plans
  are deterministic and the fingerprint covers their code; the button is
  the escape hatch.

## Verification

Start time to first chunks and time after a script reload, before and
after; images unchanged (the four reference views); script tests; a
changed landscape file and a changed vehicle file each reload as
expected (replan vs keep).

## As built

- `generators.rs`: `GeneratorEntry::fingerprint` (FNV-1a over name, seed,
  settings RON, scripts' source hash, `WORLD_VERSION`, mesher env),
  `world_cache_dir()` during `create`, `clear_world_cache`; 4 newest
  fingerprints kept. Headless gets no cache unless `FERRITE_WORLD_CACHE`.
- `crates/scripts/build.rs` registers each world with
  `register_terrain_hashed` and the transitive hash of its modules.
- `Terrain` keeps its fingerprint; when scripts reload and it is the same,
  `refresh_source` makes only the source again and the new streamer
  `adopt`s the loaded columns (`Streamer::take_loaded`/`adopt`).
- `Property::action` (a bool shown as a button; MCP `action: true`):
  Terrain's "regenerate world" clears the cache and builds anew.
- Demo: `plans.bin` (bincode) with hydrology, roads, power grid.

Measured (demo, debug editor): terrain after start ~15 s → ~4 s (plans
read in ~110 ms); script reload with an unchanged world: chunks stay
(121 of 121 columns kept, 12 ms) instead of ~12 s gone; a landscape edit
replans (~10 s), a vehicle edit keeps the world. Images after a reload
match the references within noise.

Left: the new source rebuilds the towns once (~0.4 s frame); loading the
script library itself costs a ~1.3 s frame; `plans.bin` is 88 MB (shared
`Arc`s are written out per use).
