# Asset pack: plan

Status: built 2026-11-25 on `feat/asset-pack`; see "As built" at the end.

## Why

An export copies `assets/` as it is (the demo: 262 MB of FBX, OBJ, glTF,
JPEG and PNG). Every start of the game then

- parses every model with Assimp (FBX and OBJ are slow to parse),
- decodes every JPEG/PNG to RGBA on the CPU,
- uploads them uncompressed and has the GPU build the mip chains,

and the textures take 4 bytes per texel of video memory.

## What it does

An export **cooks** the project's assets into one file, `assets.pack`,
next to the game. The game reads models and images from it and only falls
back to loose files for what is not in it.

1. **Models** (`.fbx`, `.obj`, `.gltf`, `.glb`, `.dae`, …): read with
   Assimp once, at export time, with the same post-processing the engine
   uses, and stored as the engine's own binary copy of the scene
   (`assets::cooked_scene`: meshes, materials with their properties and
   embedded textures, node tree, bones, animations). Loading one is a
   read and a few allocations; no Assimp at run time.
2. **Images** (`.png`, `.jpg`, `.jpeg`, `.tga`, `.bmp`): decoded once and
   compressed to **BC7** with every mip level (box filtered, the same as
   the GPU's own mip generation). The game uploads the blocks as they are:
   no decoding, no mip generation, a quarter of the video memory.
   BC7 for colour, normal and data maps alike (decided while planning:
   BC5 would need every shader that reads normal maps to rebuild z; BC7
   keeps three channels at a quality close to BC5).
   Exception: the terrain's textures (`assets/textures/terrain/`) are
   combined channel by channel on the CPU (`terrain::materials`), so they
   are kept as their original bytes.
3. **Everything else** under `assets/` (sidecars, materials, scatter
   graphs, structures, sounds later) stays a loose file: small, and the
   asset database scans those folders.

### File format

```
"FPAK" u32 version
u64 index offset, u64 index length
… entry data …
index: bincode Vec<Entry { path: String (project relative, '/'), kind, offset, length }>
```

Kinds: `Scene` (cooked scene), `Bc7 { width, height, mips }` (block data
of mip 0 first), `Raw` (original bytes). The game opens the pack once
(`assets::pack::open`), reads the index, and reads entries with
positioned reads from any thread.

### Engine changes

- `gfx`: formats `Bc7` and `Bc7Srgb`, `UploadFormat::Blocks` (compressed
  data as it is) in the GL and Vulkan backends.
- `renderer::texture::decode`: `Decoded` holds an `Image`: `Rgba` or
  `Bc7`. `decode_file` takes a pack entry when there is one. Uploads of
  material maps (`Texture::load_image`) take either; code that works on
  pixels (opacity into alpha, gloss into roughness, the terrain) asks for
  `into_rgba` (BC7 decoded on the CPU, only where needed).
- `assets::vfs`: `is_file`, `read_dir` (loose files and the pack's
  paths), `read`. Used where the loaders look for files today
  (`Model::read_file`, map lookup by name, texture candidates, animation
  files, outlines).
- `assets::cook` (feature `cook`, on in the editor, off in exported
  games): walks the project, cooks into `.ferrite-cache/cook/` (one file
  per source, keyed by path, size and modification time, so the next
  export reuses what did not change), and writes the pack.
- `export`: cooks, writes `assets.pack`, copies only what was not packed.

## Not now

- The editor keeps loading the source files (user's decision). A shared
  cook cache for the editor can come later on top of the same code.
- Compressing the pack as a whole (zstd): BC7 does not compress much more,
  and the cooked scenes are small next to the textures.
- Cooking the terrain's texture arrays.

## Verification

- Unit tests: scene round trip (every field), BC7 upload sizes, pack
  index and reads, the vfs listing.
- Export the demo: the pack is written, the game starts, the scene looks
  the same as with loose files (screenshots side by side), time from start
  to the menu and to the world before and after, video memory before and
  after (RAM map / diagnostics).

## As built

- `assets::cooked_scene` (write/read, every field of the Assimp scene,
  shared embedded textures, parents of nodes), `assets::pack` (format,
  reader with positioned reads, `PackWriter`), `assets::vfs` (`is_file`,
  `files_in`, `read`, `read_scene`), `assets::cook` (feature `cook`:
  parallel cooking, cache in `.ferrite-cache/cook/`, BC7 with Intel's
  ISPC encoder, basic quality, triangle-filtered mips).
- `gfx`: `Format::Bc7`/`Bc7Srgb`, `UploadFormat::Bc7` (GL
  `glCompressedTextureSubImage*`, Vulkan `BC7_*_BLOCK`).
- `decode::Image` (`Rgba` or `Bc7`), `Texture::load_image`; pixel work
  (opacity, gloss, terrain, bridges, procedural trees) asks `into_rgba`.
- The asset database finds packed sources by their pack key
  (`find_by_source` canonicalised only files on disk: the demo's building
  parts were "not an asset" in the first export).
- Terrain textures stay loose (`cook::LOOSE_IMAGE_FOLDERS`).
- An OBJ's `.mtl`, a glTF's `.bin` and `.blend` files are left out unless
  their model could not be cooked.

Measured (demo, 2026-11-25, 140 models with their maps, CPU side, release
build, `cook::tests::loose_files_against_the_pack`): loose files 3.22 s and
8.3 GB of pixels for the GPU (without mips); pack 0.53 s and 3.0 GB (with
every mip). Export: 215 files cooked (~25 s the first time, then from the
cache), pack 234 MB, 482 small files loose.
