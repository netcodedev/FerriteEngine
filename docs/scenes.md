# Scenes

A scene is a tree of entities, saved as [RON](https://docs.rs/ron) under
`scenes/` in a project. The project's `startup_scene` names the one the
editor, the server and an exported game open. The reader is
`crates/engine/src/core/scene/scene_file.rs`.

## File format

```ron
(
    version: 1,
    entities: [
        (
            id: 1001,
            name: "camera",
            position: (0.0, 0.0, 0.0),
            rotation: (1.0, 0.0, 0.0, 0.0),
            components: [
                Camera(
                    position: (0.0, 70.0, -30.0),
                    yaw_deg: 90.0,
                    pitch_deg: -25.0,
                    fov_deg: 64.0,
                    near: 0.1,
                    far: 1000.0,
                    speed: 15.0,
                    free: true,
                ),
            ],
        ),
        (
            id: 1002,
            name: "sky light",
            position: (0.0, 0.0, 0.0),
            rotation: (1.0, 0.0, 0.0, 0.0),
            components: [
                SkyLight(time_of_day: 0.38, day_length_seconds: 600.0, paused: true, cloud_coverage: 0.3),
            ],
        ),
        (
            id: 1003,
            name: "crate",
            position: (4.0, 20.0, 0.0),
            rotation: (1.0, 0.0, 0.0, 0.0),
            scale: (2.0, 2.0, 2.0),
            components: [
                Shape(kind: Cube, color: (0.6, 0.4, 0.2)),
            ],
        ),
    ],
)
```

This is the starter scene a new project gets (plus a crate), in
`crates/engine/src/core/scene/starter_scene.ron`.

- **Rotation** is a quaternion `(w, x, y, z)`.
- **Scale** is left out when it is `(1, 1, 1)`.
- **Ids** must be unique within the scene. Keep them when editing by hand.
- `settings` (optional) holds `shadow_map_size` and `debug_map_size`.
- `children` (optional) nests entities; their transforms are relative to
  the parent.

## Components

| RON | |
| --- | --- |
| `Camera(position, yaw_deg, pitch_deg, fov_deg, near, far, speed, free)` | `free` (default false): flies with WASD and the mouse in play mode |
| `SkyLight(time_of_day, day_length_seconds, paused, cloud_coverage, …)` | `clouds`, `ambient_occlusion`, `hdr`, `weather` are optional |
| `Terrain(seed, scatter: [(asset: …, density: …)])` | `density`: placements tried per 100 m × 100 m |
| `Shape(kind: Cube \| Sphere \| Cylinder, color: (r, g, b))` | sRGB, 0 to 1; one metre across |
| `Model(asset: …)` | A model asset (its id from the `.asset.ron` file) |
| `Script(name: "Lantern", data: {"radius": 5.0})` | A project script and its parameters; `data: ()` for its defaults |
| `Player` | Prefab: model, animation graph, physics capsule, bone colliders |
| `AnimationGraph(path: "…")` | Replaces the graph a prefab brought |
| `DebugController` | The debug overlay |

### Time of day

`time_of_day` runs from 0 to 1 over a day: 0.25 is sunrise (the sun on the
horizon, still dark), 0.5 noon, 0.75 sunset. Daylight is good from about
0.3 to 0.7.

## When something cannot be loaded

A component that cannot be *built* (a missing asset, a script that is not
loaded, the player prefab without its assets) becomes a placeholder that
keeps its data: the rest of the scene loads, saving loses nothing, and the
editor shows the reason. A component whose RON the engine does not know at
all makes the whole file fail to load. A file with a `version` newer than
the engine's is refused.
