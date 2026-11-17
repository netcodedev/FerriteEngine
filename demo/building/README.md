# Building demo

Building from blueprints: you don't place finished parts, you put down a
blueprint, which only shows as a see-through outline, and then build it
part by part. Stones go into a foundation, beams into walls, each one aimed
at the very spot it belongs.

```shell
cargo run -p sandbox -- demo/building
```

Or open `demo/building` in the editor and press play.

## Controls

| Input | |
| --- | --- |
| 1, 2, 3 / mouse wheel | Inventory: blueprint, stones, beams |
| Right click (blueprint in hand) | Radial menu: foundation, wall, door frame |
| Left click (blueprint) | Put the blueprint down |
| Left click (stones, beams) | Put the part in where the crosshair is |
| R | Turn a free-standing foundation by 15° |
| WASD, Space, mouse | Walk, jump, look |
| F | Fly (Space and Shift for up and down) |
| Esc | Free or catch the mouse |

## Rules

- **Foundation** (3 × 3 m): nine large blocks, then seven flagstones on top.
  Only the layer that can be built shows: a flagstone's outline appears
  once the blocks under it are in. Foundations
  snap to the side of a foundation next to them; a free-standing one sits on
  the highest ground under it, and is refused on ground that is too steep.
- **Wall** and **wall with a door frame** (3 m wide, 3 m high): two posts and
  rows of beams; the door wall has door posts, short beams beside the
  opening and a lintel. A beam's outline appears once the row below it is
  in, and beams go in only once the foundation under the wall is finished. Walls snap to the edges
  of foundations; two walls meeting at a corner share their post.
- Materials are unlimited for now.

## Scripts

| File | |
| --- | --- |
| `scripts/building_parts.rs` | Blueprints, their parts and which parts carry which, snapping, the part meshes (bevelled, rough stones and hewn beams) |
| `scripts/builder.rs` | The `Builder` script: walking, aiming, placing, drawing the parts and outlines |
| `scripts/build_hud.rs` | Crosshair, inventory, progress of the aimed blueprint, messages, radial menu |

The tests of the parts and the radial menu run with the scripts built in:

```shell
FERRITE_SCRIPTS=$PWD/demo/building/scripts cargo test -p ferrite_scripts
```
