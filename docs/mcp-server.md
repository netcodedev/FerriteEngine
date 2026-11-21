# MCP server (`ferrite-mcp`)

`ferrite-mcp` lets an AI assistant that speaks the Model Context Protocol
(Claude Desktop, Claude Code, Cursor, Zed, …) work on a Ferrite project:
create a project, write scripts, build scenes, and drive a running editor
(place and change entities, set script parameters, play, take screenshots).

It is an authoring tool. It does not link the engine and is not part of an
exported game.

## Running

```shell
cargo run -p ferrite-mcp -- path/to/project
```

or the built binary `target/debug/ferrite-mcp path/to/project`. Without an
argument it works on the current directory (and `create_project` /
`open_project` can switch later). It speaks JSON-RPC 2.0, one message per
line, on stdin and stdout.

In Claude Desktop (`~/.config/Claude/claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "ferrite": {
      "command": "/path/to/voxelengine/target/debug/ferrite-mcp",
      "args": ["/path/to/my-project"]
    }
  }
}
```

In Claude Code:

```shell
claude mcp add ferrite -- /path/to/voxelengine/target/debug/ferrite-mcp /path/to/my-project
```

## Two kinds of tools

- **File tools** work on the project's files, with or without an editor.
- **`editor_*` tools** change the scene open in a running editor. The
  editor listens on `127.0.0.1` and writes the port to `.ferrite-mcp.lock`
  in the project folder; it removes the file when it closes. Changes appear
  on the next frame and are saved with `editor_save`.

While the editor has a scene open, the file tools refuse to change that
scene's file: the editor would write its own state over the change when it
saves. Use the `editor_*` tools for it, or open another scene first.

## Tools

### Projects and scripts

| Tool | |
| --- | --- |
| `project_info` | Name, startup scene, path, whether an editor has it open |
| `create_project` | Folders, `.fp` file, a starter scene (free-flying camera, sky, terrain) as startup scene, the player prefab's assets; then works on it |
| `open_project` | Work on another project |
| `set_startup_scene` | The scene the project opens with |
| `list_scenes` / `list_assets` / `list_scripts` | What is in the project |
| `read_script` / `write_script` | `scripts/<name>.rs` (snake_case names) |
| `build_scripts` | Compile the scripts; returns every compiler error and warning |

### Scene files

| Tool | |
| --- | --- |
| `read_scene` / `scene_summary` | Raw RON, or the entity tree with component labels |
| `create_scene` | Empty, or the starter scene with `starter: true` |
| `add_entity` / `remove_entity` / `move_entity` / `rename_entity` | Entities |
| `add_component` / `remove_component` | Components, as RON |

### Live editor

| Tool | |
| --- | --- |
| `editor_status` | Mode, open scene, unsaved changes, selection, last status message, last script build message, loaded scripts, components that could not be built, whether there is a camera |
| `editor_list_entities` / `editor_get_entity` | Entities with their components and properties |
| `editor_add_entity` / `editor_remove_entity` / `editor_move_entity` / `editor_rename_entity` | Entities |
| `editor_select` | Select an entity (the inspector shows it) |
| `editor_component_catalog` | Every component kind with ready-to-use RON; every script with its parameters |
| `editor_add_component` / `editor_remove_component` | Components |
| `editor_set_property` | One property of one component: script parameters, time of day, moonlight, camera speed, shape or light colour, a particle preset, … |
| `editor_open_scene` | Open a scene (refused with unsaved changes unless `discard_changes`) |
| `editor_reload_scripts` | Build and load the scripts |
| `editor_play` / `editor_stop` / `editor_save` | Play mode, saving |
| `editor_focus` / `editor_set_view` | Point the editor camera at an entity, or place it |
| `editor_open_asset` | Open an asset (id, name or path below `assets/`) in the asset workspace; a clip opens in the clip editor |
| `viewport_screenshot` | The viewport or window as an image; scaled to 1280 px across unless `max_width` says otherwise (0: full size); PNG or JPEG |
| `editor_agent_control` | Tell the user an agent is at the controls (`active: true`, with a `note`) or gave them back (`active: false`). The editor shows a sign beside play/stop and frames the viewport. Call it before moving the camera, flying, playing or taking a series of screenshots, and switch it off when done; it ends by itself after a minute without requests |

### Several at once

`batch` runs a list of `{tool, arguments}` in order and returns each result.
It stops at the first error unless `stop_on_error: false`. Screenshots and
nested batches are not allowed in a batch.

## Setting properties

`editor_get_entity` lists each component with its index and properties.
`editor_set_property` takes the name as shown there (`"time of day"`), or
as in a scene file (`"time_of_day"`), in any case. The value is converted
to the property's kind (`3` works for a float, `"true"` for a flag; a
choice must be one of its options). The answer is the value the component
ended up with; if it ignored the change, the answer is an error.

Changes made during play mode last until play mode stops.

## Example: a scene from nothing

```jsonc
{"tool": "create_project", "arguments": {"path": "/home/me/games/lantern-keeper"}}
// The editor is started on it (cargo run -p ferrite-editor -- …), then:
{"tool": "editor_component_catalog", "arguments": {}}
{"tool": "batch", "arguments": {"operations": [
  {"tool": "editor_add_entity", "arguments": {"name": "Lantern A", "position": [8, 15, 2]}}
]}}
// → {"results": [{"result": {"id": 1234}}]}
{"tool": "editor_add_component", "arguments": {"id": 1234,
  "ron": "Shape(kind: Cylinder, color: (0.3, 0.25, 0.2))"}}
// A warm light in it (spot and directional lights point along the entity's -Y).
{"tool": "editor_add_component", "arguments": {"id": 1234,
  "ron": "Light(kind: Point, color: (1.0, 0.7, 0.4), intensity: 10.0, range: 12.0)"}}
{"tool": "editor_set_property", "arguments": {"id": 1234, "component_index": 0,
  "name": "color r", "value": 1.0}}
{"tool": "editor_focus", "arguments": {"id": 1234}}
{"tool": "viewport_screenshot", "arguments": {"max_width": 800}}
{"tool": "editor_save", "arguments": {}}
```

## Wire protocol

```
→ {"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}
← {"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18",…}}
→ {"jsonrpc":"2.0","id":2,"method":"tools/list"}
→ {"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"project_info","arguments":{}}}
← {"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"{…}"}],"isError":false}}
```

A tool's answer is JSON in a text block; a screenshot is an image block
plus a text block with its size. A tool that fails answers with
`isError: true` and the reason. JSON-RPC errors are only for bad messages
and unknown methods; notifications get no answer.

## Limits

- Paths stay inside the project: absolute paths and `..` are refused for
  scenes, and script names are plain module names.
- The server cannot start the editor. `create_project` answers with the
  command that does.
- If the editor crashed, a stale `.ferrite-mcp.lock` can be left behind;
  the `editor_*` tools then say so. Starting the editor again replaces it.

## Adding a tool

File and `editor_*` tools are in `crates/mcp/src/tools.rs`: an entry in
`catalog()`, an arm in `dispatch`, a function, a test. A new editor request
also needs a variant of `BridgeRequest`
(`crates/engine/src/core/mcp_bridge/protocol.rs`) and its handling in
`crates/editor/src/editor/bridge_handler.rs`.
