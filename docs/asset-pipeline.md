# Asset pipeline

Plan for importing files into the project as reusable assets, browsing and
editing them in the editor, and using them (instead of raw file paths)
wherever a component needs a model or a texture.

Status: phases 1–7 are done on `feat/assets`. The items under "Later" are
open. The checklist under "Phases" is kept up to date.

## Goals

- **Import**: pick a file anywhere on disk; it is copied into the project
  and registered as an asset that can be reused.
- **Asset browser**: a panel that lists the registered assets and is the
  starting point for importing, inspecting and assigning them.
- **Assets instead of files**: components refer to assets by a stable id,
  not by path. First user: the terrain's scatter layers.
- **Drag and drop**: drag an asset from the browser onto a field that
  accepts it.
- **Editable assets**: an asset has settings (for a model: scale, which
  texture each material uses, which materials are foliage) with their own
  editing view.
- **Files with many meshes**: one file can hold several things. A tree pack
  holds different trees, each with several LODs. Each tree has to be
  usable on its own, and the whole pack as one.

Not in scope for now: a binary/cooked asset format, hot reload by file
watching, dependency-aware delete, thumbnails. They are listed under
"Later".

## Terms

| Term | Meaning |
| --- | --- |
| Source file | The raw file (`.fbx`, `.gltf`, `.png`, ...) as it came from the artist. |
| Asset | A registered source file with a stable id, a name and settings. |
| Variant | One usable object inside a model asset, e.g. one tree of a pack. Has one or more LODs. |
| LOD | A detail tier of a variant. Chosen automatically by distance, never referenced on its own. |
| Asset reference | Asset id plus an optional variant name. Without a variant it means "all variants of this asset". |

## On disk

Assets live in the existing `assets/` folder (still not under version
control — it holds large binary files).

Every asset is a source file with a sidecar file next to it:

```
assets/
  models/
    Acer tree pack/
      Acer tree pack.fbx
      Acer tree pack.fbx.asset.ron     <- the model asset
      Bark_albedo.png
      Bark_albedo.png.asset.ron        <- a texture asset
```

The sidecar (`<source file name>.asset.ron`) holds the id, the name and
the settings:

```ron
(
    version: 1,
    id: "7f3a9c1e5b2d4a60",
    name: "Acer tree pack",
    source: "Acer tree pack.fbx",
    kind: Model((
        scale: 1.0,
        variants: [
            (name: "Acer_01", lods: ["Acer_01_LOD0", "Acer_01_LOD1"]),
            (name: "Acer_02", lods: ["Acer_02_LOD0", "Acer_02_LOD1"]),
        ],
        materials: [
            (name: "Bark_Mat", base_color: Some("19c0de0b2a6f4e11"), normal: None, foliage: false),
            (name: "Leaf_Mat", base_color: None, normal: None, foliage: true),
        ],
    )),
)
```

Decisions:

- **Sidecar files, no central registry.** The database is built by
  scanning `assets/` for `*.asset.ron`. A folder can be moved or copied as
  a whole and stays valid; there is no index that can go stale.
- **Ids are random 64-bit numbers**, written as 16 hex digits. They never
  change, so renaming or moving an asset does not break scenes.
- **The model's structure is cached in the sidecar** (variants, LODs,
  materials). The browser can show a pack's trees without loading the
  model, and settings can be attached to a material by name. "Reimport"
  refreshes the structure and keeps settings of things that still exist.
- **A texture slot set to `None` means "automatic"**: the model loader
  resolves it as it does today (embedded texture, file referenced by the
  material, lookup by material name). `Some(id)` overrides it with a
  texture asset. Existing models keep working without touching anything.

## Import

1. The user picks one or more files (system file dialog).
2. A file outside `assets/` is copied to `assets/models/<name>/`
   (textures: `assets/textures/`). A file already inside `assets/` is
   registered where it is.
3. For a model, the file is opened with Assimp (no GPU needed) to find
   its variants, LODs and materials. Texture files the materials refer to
   are looked up next to the source (and in `textures/` below it), copied
   along and registered as texture assets.
4. The sidecar is written and the asset appears in the browser.

Importing the same source file again returns the existing asset.

### Variants and LODs

Detection uses the node names of the file, as the scatter code already
did:

- The top-level nodes are LODs (`..._LOD0`, `high`, `low`, `billboard`,
  ...): the file is one variant with those LODs.
- Otherwise each top-level node is a variant, and the LOD-named nodes
  below it are its LODs.
- No LOD names at all: one variant with a single LOD holding everything.

The same function is used by the importer and by the loader, so the names
in the sidecar always match what gets loaded.

## Engine

New module `core::assets`:

- `AssetId`, `AssetRef` — ids and references, serialisable.
- `AssetMeta`, `AssetKind`, `ModelSettings`, `MaterialSettings` — the
  sidecar contents.
- `AssetDatabase` — scan, look up, import, save. One process-wide
  instance rooted at `assets/` (`assets::database()`); tests create their
  own on a temporary folder.
- Every saved change bumps the asset's revision, so users of the asset
  can notice and reload.

Users of assets:

- **Terrain scatter** (phase 3): a layer refers to an `AssetRef`. With a
  variant it scatters just that tree; without, all variants of the pack
  mixed. Scale, texture overrides and foliage flags come from the asset.
  A layer reloads when its asset's revision changes. Layers with a raw
  path (old scenes) still load.
- **Model component** (phase 7): shows a model asset — the variant the
  reference names, or the first one, at its most detailed LOD — with the
  asset's scale and texture overrides. Reloads when the asset is saved.
- **Player** (phase 7): its model is a model asset. Without one given it
  uses `Mannequin.fbx`, as an asset once that file is registered.
- **Animation graphs** (phase 7): a clip of a state refers to an
  animation asset; for a file with several clips the reference names the
  clip.

### Animation assets

A model file without meshes is an animation asset; its clips are listed
in the sidecar with their length. A model file that brings clips along
lists them too, and they can be used the same way.

### Old references

Scenes and graphs written before assets existed name raw files. When
they are loaded, such a name becomes an asset reference wherever the
file is registered, and is saved as one from then on. Where the file is
not registered, the name keeps working as a path.

The editor registers every importable file in `assets/` that has no
sidecar yet when it starts and on Refresh. Together with the above this
means: put files into `assets/`, start the editor, and everything refers
to assets. The game itself never writes sidecars.

### Missing assets

A component that cannot be built when a scene is loaded (its asset is
gone, the player's animations are not there) no longer fails the whole
scene. The entity gets a placeholder component that says what is wrong
and keeps the component's data, so saving the scene loses nothing and
the component comes back once the cause is fixed.

## Editor

### Asset browser (phase 4)

Docked below the viewport, resizable, can be hidden from the Window menu.

- Tree of folders and assets; a model asset expands into its variants.
- Import (file dialog) and Refresh (rescan).
- Selecting an asset shows a one-line summary. Opening it in the asset
  editor comes with phase 6.

### Using assets in fields (phase 4 and 5)

- A scatter layer shows the asset's name instead of a path.
- "Add model" and the button on a layer open an asset picker (model
  assets and their variants) instead of a file dialog.
- A model asset dropped on the viewport becomes an entity showing it;
  dropped on the model row of an entity's model component it replaces the
  model. "Add component → Model" asks for the asset.
- Drag and drop (phase 5): dragging a row out of the browser starts a
  drag with an asset reference as payload. Fields that accept it
  highlight while the drag is over them; dropping assigns it. Dropping on
  a terrain's "Add model" row, or anywhere else on its inspector, adds a
  layer. Implemented in the editor:
  the drag payload lives in the editor state, fields register their drop
  rectangles every frame, and the editor layer resolves the drop.

### Asset editor (phase 6)

A third workspace next to Scene and Animation, opened for one asset with
the Edit button of the asset browser or a double click on the asset. The
settings take the left column, the preview the place of the viewport; the
asset browser stays, so textures can be dragged onto a material.

- Model: 3D preview with orbit camera and a variant/LOD selector; name
  and scale; per material the texture slots (drop a texture asset or pick
  one) and the foliage flag.
- Texture: the image and its size. Sampling settings come later.
- Edits are made on a working copy, which the preview shows right away.
  Save (Ctrl+S) writes the sidecar and bumps the revision, so the scene
  updates without a reload. Revert discards the edits.
- Leaving the workspace keeps unsaved edits in memory. Opening another
  asset is refused until they are saved or reverted.

## Phases

Each phase is one or a few commits and leaves the editor usable.

- [x] **0. Plan** — this document.
- [x] **1. Asset database** — ids, sidecar format, scan, save, lookup.
  Unit tests.
- [x] **2. Model inspection and import** — GPU-free detection of
  variants, LODs and materials; import with copying and texture
  registration. Unit tests with a generated glTF.
- [x] **3. Scatter uses assets** — asset references in scatter layers and
  scene files; variant filter, scale, texture overrides, foliage flag;
  reload on revision change.
- [x] **4. Asset browser** — panel, layout, import, refresh; asset picker
  for scatter layers.
- [x] **5. Drag and drop** — from the browser onto scatter layers and
  "Add model".
- [x] **6. Asset editor** — workspace with preview and material editing.
- [x] **7. More users** — model component, player model, animation clips
  in graphs; automatic registration and upgrade of old references;
  placeholders for components that cannot be built.

## Later

- Reimport: refresh a model asset's variants and materials from its
  source file, keeping the settings of what still exists.
- Ask before quitting with unsaved asset edits.
- Thumbnails rendered from the preview.
- Rename, move and delete in the browser, with a check for scenes that
  still refer to the asset.
- File watching: reimport when a source file changes on disk.
- Texture settings (sRGB, filtering, wrap) and an opacity slot.
- A cooked cache so large models do not go through Assimp on every start.

## Open questions

- Should `assets/` (or at least the sidecars) go under version control,
  e.g. with Git LFS? Scenes refer to asset ids, and those only exist in
  the sidecars.
- Animation graphs and scenes are still project files referred to by
  path (`animations/`, `scenes/`), not assets. They are documents the
  editor writes rather than imported files; should they show up in the
  asset browser anyway?
- The voxel terrain backend loads `assets/grass.png` and
  `assets/stone.png` directly. It is not used by any scene at the moment.
