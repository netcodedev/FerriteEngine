# Scripting

Scripts are Rust files in a project's `scripts/` folder. The engine compiles
them into a library (the `ferrite_scripts` crate of the engine's workspace,
built with `FERRITE_SCRIPTS` pointing at the folder) and loads it. The
editor rebuilds and reloads the library when a file in the folder changes;
the development server does the same on start, and an exported game has
the scripts compiled in.

## Writing a script

A script is a struct with `#[derive(Script)]`, a `Default` and an
`impl Behaviour`. Its public fields are its parameters: saved in scenes and
shown in the inspector. Other fields are the script's own state.

```rust
use ferrite::prelude::*;

/// Turns its entity around the vertical axis.
#[derive(Script)]
pub struct Rotator {
    /// Degrees per second.
    pub speed: f32,
    angle: f32,
}

impl Default for Rotator {
    fn default() -> Self {
        Self { speed: 45.0, angle: 0.0 }
    }
}

impl Behaviour for Rotator {
    fn update(&mut self, _scene: &mut Scene, entity: &mut Entity, delta_time: f64) {
        self.angle += self.speed * delta_time as f32;
        entity.set_local_rotation(Quaternion::from_angle_y(Deg(self.angle)));
    }
}
```

Save it as `scripts/rotator.rs`. The file name becomes the module name, so
it has to be snake_case; nothing has to be registered anywhere. Do not
derive `Serialize`/`Deserialize` yourself: the derive generates what saving
needs, and the scripts crate has no direct `serde` dependency.

A public field marked `#[script(skip)]` is not a parameter.

## Parameters

Parameters can be numbers (shown as float or whole-number fields), `bool`,
`String`, and short arrays of those (`[f32; 3]` shows as `axis x`,
`axis y`, `axis z`). The inspector shows the field name with `_` as a space
(`move_speed` → `move speed`).

## The `Behaviour` trait

Every method is optional:

| Method | When |
| --- | --- |
| `update(scene, entity, delta_time)` | Every frame of the game (play mode) |
| `update_in_editor(scene, entity, delta_time)` | Every frame while the scene is edited, not played |
| `render(scene, entity, view_projection, parent_transform)` | Drawing |
| `render_transparent(...)` | After everything opaque is drawn (overlays, transparency) |
| `handle_event(glfw, window, event)` | Window events (keys, mouse) |
| `on_remove(physics)` | When the entity leaves the scene: release what it owns (physics bodies) |

## Finding other entities and scripts

During an update, the entity being updated is taken out of the scene, so a
script sees every *other* entity:

```rust
// Every Lantern script in the scene, with its entity.
let lit = scene.scripts::<Lantern>().iter().filter(|(_, l)| l.lit).count();

// One entity by name, and a script on it.
if let Some(sun) = scene.find_entity_mut("Sun") { /* … */ }
let manager = scene.find_entity("Sun").and_then(|e| e.script::<LanternManager>());

// A script on a known entity, to change it.
if let Some(lantern) = scene.script_mut::<Lantern>(&id) { lantern.lit = true; }
```

Engine components are reached the same way, e.g.
`entity.get_component_mut::<ShapeComponent>()` or
`scene.get_active_camera()`.

## Reloading

The editor watches `scripts/`. When a file changes it builds the scripts in
the background and, if the build works, loads the new library and rebuilds
every script component from its saved parameters. The status bar says what
happened (`Scripts reloaded (3)`, or `Scripts: <first compiler error>`). A
failed build leaves the previous library loaded. Ctrl+R rebuilds by hand.

The script library must be built against the engine library the editor
runs with. If the engine is rebuilt while the editor runs, the editor says
"the engine was rebuilt; restart the editor to load the new scripts". The
editor's own script builds no longer cause that: variables `cargo run`
passes to the editor (`CARGO_MANIFEST_DIR`, `CARGO_PKG_*`, …) used to reach
the nested build and make a dependency's build script rerun, which rebuilt
the engine; they are now removed before building.

## Running without the editor

- `cargo run -p ferrite-server -- <project>` plays the project's startup
  scene without a window, with its scripts built and loaded.
- `cargo run -p sandbox` is a hand-written world using the engine directly.

## Multiplayer

`ferrite::core::net` is available to scripts. World changes go through
named channels:

- `request(channel, bytes)` / `request_value(channel, &value)`: ask for an
  event.
- `take_events(channel)` / `take_values::<T>(channel)`: the events to apply,
  every frame.
- `moderate(channel)` makes requests wait for the authority, whose scripts
  read them with `take_requests` / `take_request_values` and accept them
  with `publish` / `publish_value`.

See [multiplayer.md](multiplayer.md).
