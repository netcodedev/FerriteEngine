# RAM map

Status: built (2026-11-23).

## Why

Driving around the demo for a long time, the editor's resident memory
grows (about 3 GB to 6 GB). The process's total says nothing about what
grows: a leak (made and never freed), a cache without a bound (meant, but
endless), or malloc keeping freed memory (no leak at all). A snapshot of
what holds how much, taken twice, tells them apart: what grew is at the
top of the comparison.

## Use

- **Ctrl+Shift+M** in the editor (in edit and play mode, also while
  driving) takes a snapshot, saves it and compares it with the one before.
  The status bar says by how much the process grew and what grew most;
  the Statistics panel's Memory section keeps the change and the three
  rows that grew most until the next snapshot.
- **MCP:** `editor_memory_snapshot` (the same, returns the comparison or,
  for the first, the largest rows) and `editor_memory_diff` (any two
  saved snapshots; the last two by default).
- **Files:** `<log folder>/memory/<UTC time>[_label].ron`, and next to each
  a `.txt` with the whole comparison with the snapshot before. The newest
  40 are kept. The log folder is `~/.local/share/<app>/logs` on Linux
  (`FERRITE_LOG_DIR` overrides it).

Typical: start, drive a minute (chunks around loaded), snapshot; drive
ten minutes, snapshot; read the top of the comparison.

## What a snapshot holds

Rows of `group`, `name`, an optional count and bytes (`core::memory`).

| group | rows | from |
|---|---|---|
| `process` | resident memory and its parts, swap, peak; resident bytes per kind of mapping (malloc heap, anonymous, graphics driver, libraries, files, stacks) | `/proc/self/status`, `/proc/self/smaps` (Linux) |
| `heap` | Rust's allocations alive (bytes, blocks); malloc's use over all arenas, its own blocks, and freed memory it keeps; the Rust heap no holder below explains | the counting global allocator (`memory::heap::Counting`), glibc's `mallinfo2` |
| `gpu` | the Vulkan allocator's allocations by name (buffers by GPU / host-visible, textures by kind, acceleration structures, ...), its blocks and their unused part, freed buffers kept for reuse; the driver's used and total video memory | `gpu_allocator`'s report (`Device::memory_items`) |
| engine parts (`terrain`, `scatter`, `rays`, ...) | the long-lived holders by count and estimated bytes | `Scene::memory_report` |

Reading it:

- **Rust heap alive grows** with the resident memory: something in Rust
  holds more. The engine rows say what; if they stay flat, "no holder
  explains" grows: a holder not reported yet (heaptrack finds it).
- **Rust heap flat, "malloc freed but kept" or the resident memory grows:**
  malloc does not give freed memory back (fragmentation): `malloc_trim` or
  another allocator, not a leak hunt.
- **malloc in use grows beyond Rust's heap:** C libraries or the driver
  (malloc'd memory the Rust counter does not see).
- **`mapped: graphics driver mappings` or a `gpu` row grows:** GPU memory,
  host-visible buffers count into the process too.

The counting allocator costs two relaxed atomic additions per allocation,
spread over 16 cache lines by the calling thread's stack.

## Engine holders

`Scene::memory_report` walks the entities: every component is counted by
type (`components` rows: a kind whose count only grows stands out), and
each may add rows of its own (`Component::memory_report`, nothing by
default). Then physics and the ray tracing caches. Holders owned anywhere
(scripts' own components too) count themselves with a `memory::Tally` on a
static `memory::Counter` (`tally::ALL`).

| group | rows | where |
|---|---|---|
| `scene`, `components` | entities; components by type | `Scene::memory_report` |
| `terrain` | chunks and their CPU mesh copies (0 bytes once uploaded: more means a release was missed); streaming columns in flight and loaded; horizon heights and meshes | `TerrainChunk`, `Streamer`, `Horizon` |
| `scatter` | instance lists of chunk layers and of detail cells; their colliders; planting points ready and asked for; felled instances remembered | `ScatterInstances`, `Planting`, `Scatter` |
| `physics` | bodies and colliders (bytes a rough per-item estimate); triangle mesh shapes, each once, with an estimate for the BVH; the query index | `PhysicsEngine`, `RayIndex` |
| `rays` | BLAS cached, skinned meshes posed, this frame's instances (GPU bytes are in `gpu`) | `raytracing::memory_report` |
| `models` | every model's CPU geometry (assimp's scene and the meshes' vertices and indices, kept after upload) | `Tally` in `Model` |
| `world` (the demo) | the generator's column and macro tile caches | `TerrainSource::memory_report` |

Not reported (in "no holder explains"): the world plans (hydrology, roads,
power, towns), assets, materials, scripts' other state.

## First finding (2026-11-23)

Three minutes of flying over the demo with every ray tracing option on:
resident +1.67 GB, Rust heap alive +22 MB, malloc's "freed but kept"
+1.64 GB, peak resident 5.0 GB. The growth is glibc malloc keeping what
the chunk and scatter threads freed (an arena per thread), not a leak; the
engine's holders stayed flat or were bounded (column cache at its 512).
The GPU allocator kept 16 blocks with 1.09 GB unused in them (video
memory, not RAM).

**Fixed** (`memory::heap::tune_malloc`, from `diagnostics::install`): glibc
raises its mmap threshold up to 32 MB as big blocks are freed, so chunk
meshes and collider and scatter lists landed in the worker threads' arenas
and stayed there. A fixed threshold of 256 KB (`mallopt(M_MMAP_THRESHOLD)`)
gives them their own mmap, returned on free. Same flight, same frame rate:

| | before | periodic `malloc_trim` | fixed threshold |
|---|---|---|---|
| resident at start | 2.50 GB | 1.92 GB | 1.49 GB |
| growth in 3 min | +1.67 GB | +0.30 GB | +0.11 GB (world caches filling) |
| peak resident | 5.0 GB | 4.5 GB | 3.1 GB |

A second flight over new ground right after grew it by 64 MB, the macro
tile cache filling towards its bound. `malloc_trim` every 5 s was dropped:
it held the arenas' locks for up to 42 ms.
