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
writes the histories and `load_world(path)` carries on from them (all of
them: the per-channel queue limit of 10,000 events does not apply to a
loaded world). The server logs a warning when the history passes 8 MiB,
and again at every doubling: a game that never snapshots makes every join
slower and every save bigger.

## Limits

- **Joining.** A player who joins gets the `Welcome` at once and then the
  history, handed to its reliable channel a bit per update (at most
  256 KiB, and only while the channel has room), so a long history takes
  a while but never overflows the connection. Events made meanwhile are
  queued behind it, so the order stays the same.
- **Channel memory.** The reliable channels hold up to 32 MiB sent but
  not yet acknowledged (renet's default is 5 MiB; a peer whose channel
  overflows is disconnected). The scene a server shares has to fit.
- **Requests.** A client may send requests of at most 64 KiB, and at most
  240 requests and 1 MiB of them per second. More are dropped, with a
  warning in the server's log.

The messages themselves are unchanged (the protocol id is the same).

## Connecting

`net::host(port)` makes the game a server, `net::connect(address)` joins
one, `net::disconnect()` leaves.

Who a player is (where the server puts it when it comes back) is kept in
the project's `saves/player.id`, locked while the game runs. A second game
of the same project on the same machine (play mode in the editor and a
player started with `--connect`) is another player, kept in
`saves/player-2.id`, and so on. `is_server`, `is_client`, `local_id` and
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
