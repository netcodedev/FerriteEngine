# Audio

Sound for games and the editor, built on [kira](https://crates.io/crates/kira)
0.12 (`core::audio`). Without an audio device, on a dedicated server
(`headless`) and in tests, every call does nothing.

## Buses

Every sound plays on one bus of the mixer, each with its own volume, under
the master volume:

| Bus | For |
| --- | --- |
| `Effects` | Things that happen: engines, tools, steps, impacts |
| `Ambience` | What is always around: wind, fire, birds, rain |
| `Music` | Music |
| `Voice` | Speech and radio |

`audio::set_volume(Some(Bus::Music), 0.7)` sets a bus (0 to 1),
`audio::set_volume(None, 0.8)` everything. Changes glide over 30 ms.

## Playing sounds

```rust
use ferrite::core::audio::{self, Emitter, Play};

// Once, where something happened (heard up to 50 m away).
audio::play("assets/sounds/impact_heavy.ogg", Play::at(point).volume(0.8).pitch(1.2).range(2.0, 50.0));

// Once, everywhere alike (a menu click).
audio::play("assets/sounds/click.ogg", Play::default());

// Over and over for as long as it is kept: an engine.
let mut engine = Emitter::new("assets/sounds/truck_engine_loop.ogg", Play::at(point).range(3.0, 120.0));
engine.set_position(point); // every frame
engine.set_pitch(1.4);
engine.set_volume(0.9);
drop(engine); // fades out over 120 ms
```

Paths are project relative (`.ogg`, `.wav`, `.flac`, `.mp3`) and are read
through the virtual file system, so an exported game finds them in its
asset pack. A file is decoded once and kept.

A positional sound (`Play::at`) is at full volume up to `range.near`
metres from the listener and fades out (an out-square curve) to nothing
at `range.far`; which ear hears it more follows the listener's turn. A
sound without a place is heard the same everywhere.

## The listener

Every frame `Scene::update` puts the listener at the scene camera's eye,
looking where it looks. Following the camera alone does not open the
audio device: the first sound does. `audio::set_listener` moves it
elsewhere (after the scene update).

## Audio sources in scenes

The **Audio source** component (Add component → Audio source) plays a sound
where its entity is, without code: the sound file (a choice of the
project's sound assets), bus, volume, pitch, near and far range, looping or
once when the game starts, positional or not. It is quiet while the scene
is edited and starts when the game runs.

```ron
AudioSource(
    settings: (
        sound: "assets/sounds/campfire_loop.ogg",
        bus: Ambience,
        volume: 0.8,
        range: (near: 1.5, far: 30.0),
    ),
),
```

## Sound assets

`.ogg`, `.wav`, `.flac` and `.mp3` files are assets of the kind *Sound*:
the asset browser lists them under **Sounds**, and opening one plays it
once. Imported from outside the project, they are copied to `sounds/`.

## Status

`audio::status()` tells whether there is sound, how many sounds were
played since the start and how many emitters there are; the editor's MCP
`editor_status` shows it under `audio`.

## Not yet

- Streaming long files (music) instead of decoding them whole.
- Occlusion (walls between source and listener), reverb, Doppler.
- A preview with play and stop in the asset editor.
