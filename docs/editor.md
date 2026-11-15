# Editor

```shell
cargo run -p ferrite-editor -- path/to/project
```

Without a project in the working directory or on the command line, the
editor starts with a launcher to open or create one. A new project gets a
starter scene (a free-flying camera, a sky light and a terrain) as its
startup scene, and the player prefab's model and animations copied from the
engine's `assets` folder.

## Layout

Panels are docked and resizable; the arrangement and the window size are
saved in the project's `.fp` file.

| Panel | |
| --- | --- |
| Viewport | The scene, with gizmos for the selection |
| Hierarchy | The entity tree; select entities, add new ones |
| Inspector | The selected entity's components and their properties; add and remove components |
| Asset browser | The project's assets; drag a model into the viewport to place it |
| Statistics | Frame time and the time spent updating, rendering and drawing the UI |

## Viewport camera

Hold the right mouse button and use WASD to move, E and Q to go up and
down, Shift to move faster. F frames the selected entity.

## Keys

| Keys | Action |
| --- | --- |
| Left click | Select an entity |
| W / E / R | Move / rotate / scale gizmo |
| F | Focus the selected entity |
| Escape | Clear the selection |
| Ctrl+D / Delete | Duplicate / delete the selection |
| Ctrl+P | Enter or leave play mode |
| Ctrl+R | Rebuild and reload the scripts |
| Ctrl+N / Ctrl+O | New scene / open a scene (asks about unsaved changes) |
| Ctrl+S / Ctrl+Shift+S | Save / save as |

There is no undo yet.

## Components

"Add component" in the inspector offers Camera, Sky light, Terrain, Debug
controller, Shape, and every loaded script. Removing a component calls its
`on_remove`, so physics bodies leave the world with it.

- **Camera**: with "free fly" on it flies in play mode with WASD or the
  arrow keys, Space/E and Shift/Q for up and down, and the mouse to look
  around while the game holds the cursor. A player controller turns that
  off while it drives the camera itself.
- **Shape**: a cube, sphere or cylinder in one colour, one metre across
  (the entity's scale sizes it). For blocking out a level, or standing in
  for a model that does not exist yet.

## Play mode

Ctrl+P saves the scene in memory, switches from the editor camera to the
scene's own camera, turns physics on and captures the mouse. Without a
camera in the scene there is nothing to see; the status bar says so.
Leaving play mode restores the scene as it was.

## Animation graphs

An entity with an Animation component has **Edit graph** in the inspector,
which opens the node-based graph editor with a live preview.

## The MCP bridge

While a project is open, the editor listens for an MCP server on
`127.0.0.1` and writes the port to `.ferrite-mcp.lock` in the project
folder (removed when the editor closes). See [mcp-server.md](mcp-server.md).
