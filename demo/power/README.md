# Power demo

Electricity by hand: put up producers (solar panels, wind turbines,
generators), batteries and consumers (lamps, floodlights, heaters), and join
them with cables you draw yourself, port to port. A cable is fastened to
walls, floors or the ground wherever you click on the way, every span sags as
far as the mouse wheel says, and a cable is only as long as its reel. Poles
along the road carry an overhead line; the box on each pole lets you branch
power off it.

```shell
cargo run -p sandbox -- demo/power
```

Or open `demo/power` in the editor and press play.

## Controls

| Input | |
| --- | --- |
| 1 … 9 | Cable, solar panel, wind turbine, generator, battery, lamp, floodlight, heater, pole |
| Left click (cable in hand) | On a port: start a cable. While drawing: fasten it where the crosshair is, or join it to a port |
| Mouse wheel (while drawing) | How far the next span sags (up pulls it tighter); otherwise it picks the item |
| Shift (while drawing) | Hold the next span exactly upright or level (whichever is closer) |
| Right click (while drawing) | Take back the last point; with none, drop the cable |
| Left click (device in hand) | Put the device up where the crosshair is |
| R | Turn the next device by 15° |
| E | Switch the device under the crosshair on or off (generator, lamp, floodlight, heater) |
| X | Take down the device or cable under the crosshair |
| T | Time lapse (30 ×) |
| G | Next weather: clear, rain, snow |
| WASD, Space, mouse | Walk, jump, look |
| F | Fly (Space and Ctrl for up and down) |
| Esc | Free or catch the mouse |

While a cable is in hand every port nearby shows as a yellow dot (green: a
cable is already on it). The next span is drawn as a preview, blue when it
can go there, red with the reason when not.

## Devices

| Device | Watts | |
| --- | --- | --- |
| Solar panel | 400 W peak | Depends on the sun's height and the panel's facing, clouds, rain, falling and lying snow, and on shadows: rays from nine points on the panel toward the sun find what is in the way (terrain, buildings, other devices, trees) |
| Wind turbine | 3,000 W | Starts at 3 m/s, full power from 12 m/s, turns out of the wind above 25 m/s. The wind gusts and blows harder in rain and snow |
| Generator | 2,500 W | While switched on (E); fuel is unlimited for now |
| Battery | 5 kWh, charges and discharges at up to 3,000 W | Starts half full |
| Lamp | 60 W | On floors, walls and ceilings |
| Floodlight | 400 W | |
| Heater | 2,000 W | |
| Pole | Street lamp 100 W, at night | Two insulators on top for the overhead line, a box at the bottom to branch off |

## The grid

Every device joined by any chain of cables is one network (all ports of one
device are the same node). In each network, every frame:

1. The producers' power goes to the consumers.
2. What is left charges the batteries, as fast as they can take it. The rest
   is not made: an idle panel or turbine shows less than it could.
3. When the producers fall short, the batteries make up the rest.
4. When that is not enough either, every consumer gets the same share of what
   there is: lamps dim, the HUD shows how much is missing.

Energy runs on game time: with the sky's day length of 20 minutes one real
second is 72 game seconds (30 times that with the time lapse), so a battery
fills or empties within minutes.

## Cables

- One cable can be up to 50 m long (`cable_length` on the script), measured
  along its sag.
- A span's sag is a share of its horizontal length, so it stays the same when
  the mouse moves; spans up a wall do not sag. The overhead line between the
  poles is not bound to the reel.
- A span may not run through anything (it may touch where it is fastened):
  too much sag over the ground turns the preview red.
- With Shift the new point is moved straight above/below or level with the
  last point; it still has to lie on a surface.

## The scene

The demo puts up a small shed (to fasten cables to), a line of six poles
along a future road with their overhead line, and a wind turbine with a
battery at the first pole that feed the line. The street lamps come on at
dusk. The rest is yours to build.

## Scripts

| File | |
| --- | --- |
| `scripts/electrician.rs` | The `Electrician` script: walking, aiming, putting up devices, drawing cables, the grid each frame, drawing, lights |
| `scripts/devices.rs` | The devices: watts, ports, looks (meshes made in code, plain colours) and collision boxes |
| `scripts/cable.rs` | Spans, sag, Shift lock, cable length |
| `scripts/grid.rs` | Networks from cables, and how power is shared out |
| `scripts/sources.rs` | Solar output (sun, weather, shadow) and wind |
| `scripts/power_hud.rs` | Crosshair, inventory, device and network panels, cable length and sag, time and weather |

The tests run with the scripts built in:

```shell
FERRITE_SCRIPTS=$PWD/demo/power/scripts cargo test -p ferrite_scripts
```

## Not done yet

Fuel for the generator, switches and fuses, cables that can be picked up
again with their length back, saving what was built in the scene, multiplayer.
