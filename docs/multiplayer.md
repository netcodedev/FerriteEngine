# Multiplayer

The network code (`ferrite::core::net`) is always part of the engine. A
game runs offline, as a client of a server, or is the server itself
(`ferrite-server`, which plays the scene without a window or GPU). Scripts
use the same calls in every case.

## Players

Every peer sends its player's position, rotation and animation inputs many
times a second; everyone else sees a copy (a `RemotePlayer` component) that
follows it and jumps when it falls too far behind. Movement is decided by
the player's own game, so it never lags for the one who moves.

## World events

What changes the world goes through named channels:

```rust
use ferrite::core::net;

// Ask for a change (offline it becomes an event at once).
net::request_value("build", &action);

// Every frame: apply what happened, own requests included.
for action in net::take_values::<BuildAction>("build") {
    apply(&action, scene);
}
```

To have requests checked first, the game calls `net::moderate("build")`.
Requests then wait for the authority (the server, or the game itself
offline), whose scripts read them and accept the good ones:

```rust
if net::is_authority() {
    for (_from, action) in net::take_request_values::<BuildAction>("build") {
        if allowed(&action, scene) {
            net::publish_value("build", &action);
        }
    }
}
```

The byte versions are `request`, `take_events`, `take_requests` and
`publish`.

## Saving the world

The server keeps every event of a channel, so a player who joins later gets
them all first. `replace_history` / `replace_history_value` replace a
channel's history with a snapshot to keep it short; `save_world(path)`
writes the histories and `load_world(path)` carries on from them.

## Connecting

`net::host(port)` makes the game a server, `net::connect(address)` joins
one, `net::disconnect()` leaves. `is_server`, `is_client`, `local_id` and
`players` tell a script where it stands.

## On the wire

`crates/engine/src/core/net/protocol.rs`, over renet (UDP):

| Direction | Message |
| --- | --- |
| to the server | `Request { channel, data }`: a world event asked for |
| to the server | `State(PlayerState)`: this peer's player |
| to a client | `Welcome { scene, returning, index }`: the first message |
| to a client | `Event { channel, data }`: an event to apply |
| to a client | `Players([(id, state)])`: every player |
| to a client | `Left(id)`: a player left |

## Running a server

```shell
cargo run -p ferrite-server -- path/to/project
```

It listens on UDP port 7777 and saves the world to `saves/server.sav`.
