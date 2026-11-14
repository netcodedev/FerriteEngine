//! Multiplayer, for any game made with the engine.
//!
//! A game is played offline, as a client of a server, or is the server
//! (`ferrite-server`, which runs without a window or GPU). What the engine
//! does by itself once connected:
//!
//! - **Players.** Every peer sends where its player is, how it is turned
//!   and the inputs of its animation graph; everyone else sees a copy of
//!   that player (a `RemotePlayer`) moving and animating the same way.
//!   Movement is decided by the player's own game (client authority), so
//!   it never lags for the one who moves.
//! - **World events.** What changes the world (a game's building, felling,
//!   opening a door, ...) goes through `request` on a named channel. The
//!   server orders the requests into events, sends them to every client and
//!   keeps them, so a client that joins later gets all of them first and
//!   sees the world as it is. Offline a request is an event at once. Every
//!   peer applies the events of its channels (`take_events`), its own ones
//!   too: one way to change the world, whatever the mode.
//!
//! - **Judging requests.** A game can `moderate` a channel: its requests
//!   then wait for the authority (the server, or the game itself offline),
//!   whose scripts check them against the world (`take_requests`) and
//!   `publish` the ones they accept. The same code runs offline and on a
//!   server, so a game has one way to change its world.
//! - **Saving.** What a server keeps of a channel can be replaced by a
//!   snapshot (`replace_history`), so the history stays short; that history
//!   is what a player who joins gets and what `save_world` writes and
//!   `load_world` carries on.
//!
//! `Scene::update` runs all of this; a game only requests and applies its
//! own events, and offers a way to `host` or `connect`.

mod players;
mod protocol;

use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::Path,
    net::{SocketAddr, ToSocketAddrs, UdpSocket},
    sync::{LazyLock, Mutex, MutexGuard},
    time::{Duration, Instant, SystemTime},
};

use renet::{ConnectionConfig, DefaultChannel, RenetClient, RenetServer, ServerEvent};
use renet_netcode::{
    ClientAuthentication, NetcodeClientTransport, NetcodeServerTransport, ServerAuthentication,
    ServerConfig,
};
use serde::{de::DeserializeOwned, Serialize};

use super::scene::{scene_file::SceneFile, Scene};
use protocol::{ToClient, ToServer};

pub use players::{PlayerState, RemotePlayer};

/// A connected player, as the server numbers it.
pub type ClientId = u64;

/// The port a server listens on unless told otherwise.
pub const DEFAULT_PORT: u16 = 7777;
/// Most players on one server.
pub const MAX_PLAYERS: usize = 16;
/// Peers with another protocol cannot connect: change it when the messages
/// change.
const PROTOCOL_ID: u64 = 0x4645_5252_4954_4501;
/// How often players are sent, per second.
const SEND_RATE: f32 = 20.0;
/// Connecting gives up after this long.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);

/// Where the game stands.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Offline,
    /// This is the server; this many players are connected.
    Hosting { players: usize },
    Connecting,
    Connected,
    /// Connecting failed or the connection was lost; offline again.
    Failed(String),
}

struct Server {
    server: RenetServer,
    transport: NetcodeServerTransport,
    /// Every event so far, in order, for players who join later.
    log: Vec<(String, Vec<u8>)>,
    /// Latest state of each player.
    players: HashMap<ClientId, PlayerState>,
    left: Vec<ClientId>,
    /// The scene the server plays (`share_scene`), sent to who joins.
    scene: Option<String>,
    /// Who each connected player is (`player_key`).
    keys: HashMap<ClientId, u64>,
    /// Where every player that was here was last, by who they are: kept
    /// when they leave and saved with the world.
    last_seen: HashMap<u64, PlayerState>,
    /// Players that joined so far.
    joined: u32,
}

struct Client {
    client: RenetClient,
    transport: NetcodeClientTransport,
    id: ClientId,
    started: Instant,
    /// Latest state of the other players.
    players: HashMap<ClientId, PlayerState>,
    left: Vec<ClientId>,
    /// Arrived, not carried out yet.
    welcome: Option<Welcome>,
}

/// What the server said when this client joined, to be carried out on the
/// scene.
struct Welcome {
    scene: Option<String>,
    returning: Option<PlayerState>,
    index: u32,
}

enum Role {
    Offline,
    Server(Box<Server>),
    Client(Box<Client>),
}

struct Net {
    role: Role,
    /// Events to apply, by channel.
    events: HashMap<String, VecDeque<Vec<u8>>>,
    /// Requests on moderated channels, for the authority to judge: who
    /// asked (`None`: this process) and what.
    requests: HashMap<String, VecDeque<(Option<ClientId>, Vec<u8>)>>,
    /// Channels whose requests the game judges (`moderate`).
    moderated: HashSet<String>,
    /// Why the last connection ended, until something else happens.
    failure: Option<String>,
    last_update: Option<Instant>,
    since_send: f32,
}

static NET: LazyLock<Mutex<Net>> = LazyLock::new(|| {
    Mutex::new(Net {
        role: Role::Offline,
        events: HashMap::new(),
        requests: HashMap::new(),
        moderated: HashSet::new(),
        failure: None,
        last_update: None,
        since_send: 0.0,
    })
});

fn net() -> MutexGuard<'static, Net> {
    NET.lock().unwrap_or_else(|e| e.into_inner())
}

fn now() -> Duration {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
}

fn encode<T: Serialize>(value: &T) -> Vec<u8> {
    bincode::serialize(value).expect("network messages always serialize")
}

/// Be the server, listening on `port` on every interface.
pub fn host(port: u16) -> Result<(), String> {
    let address = SocketAddr::from(([0, 0, 0, 0], port));
    let socket = UdpSocket::bind(address).map_err(|e| format!("cannot listen on {address}: {e}"))?;
    let config = ServerConfig {
        current_time: now(),
        max_clients: MAX_PLAYERS,
        protocol_id: PROTOCOL_ID,
        public_addresses: vec![address],
        authentication: ServerAuthentication::Unsecure,
    };
    let transport = NetcodeServerTransport::new(config, socket).map_err(|e| e.to_string())?;
    let mut net = net();
    net.role = Role::Server(Box::new(Server {
        server: RenetServer::new(ConnectionConfig::default()),
        transport,
        log: Vec::new(),
        players: HashMap::new(),
        left: Vec::new(),
        scene: None,
        keys: HashMap::new(),
        last_seen: HashMap::new(),
        joined: 0,
    }));
    net.events.clear();
    net.requests.clear();
    net.failure = None;
    Ok(())
}

/// On the server: the scene it plays, as a scene file's text. Every player
/// who joins plays in it instead of in its own (the map, its seed, what is
/// placed in it), so server and players always share one world.
pub fn share_scene(scene_file: String) {
    if let Role::Server(server) = &mut net().role {
        server.scene = Some(scene_file);
    }
}

/// Who this player is, the same on every connection: a number made once
/// and kept in `saves/player.id` (in the project). The server remembers
/// players by it, e.g. where they were when they left.
fn player_key() -> u64 {
    let path = Path::new("saves").join("player.id");
    if let Some(key) = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| u64::from_str_radix(text.trim(), 16).ok())
    {
        return key;
    }
    let key = now().as_nanos() as u64 ^ (std::process::id() as u64).rotate_left(40);
    let _ = std::fs::create_dir_all("saves");
    if let Err(e) = std::fs::write(&path, format!("{key:016x}\n")) {
        log::warn!("net: cannot keep {}: {e}", path.display());
    }
    key
}

/// Connect to the server at `address` ("host" or "host:port"). Returns at
/// once; `status` tells when it is connected.
pub fn connect(address: &str) -> Result<(), String> {
    let address = address.trim();
    let with_port = if address.contains(':') {
        address.to_string()
    } else {
        format!("{address}:{DEFAULT_PORT}")
    };
    let server_addr = with_port
        .to_socket_addrs()
        .map_err(|e| format!("{address}: {e}"))?
        .find(SocketAddr::is_ipv4)
        .ok_or_else(|| format!("{address}: no IPv4 address"))?;
    let socket = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
    let id = now().as_nanos() as u64 ^ (std::process::id() as u64) << 32;
    // Who this player is travels with the connection request.
    let mut user_data = [0u8; renet_netcode::NETCODE_USER_DATA_BYTES];
    user_data[..8].copy_from_slice(&player_key().to_le_bytes());
    let authentication = ClientAuthentication::Unsecure {
        protocol_id: PROTOCOL_ID,
        client_id: id,
        server_addr,
        user_data: Some(user_data),
    };
    let transport =
        NetcodeClientTransport::new(now(), authentication, socket).map_err(|e| e.to_string())?;
    let mut net = net();
    net.role = Role::Client(Box::new(Client {
        client: RenetClient::new(ConnectionConfig::default()),
        transport,
        id,
        started: Instant::now(),
        players: HashMap::new(),
        left: Vec::new(),
        welcome: None,
    }));
    net.events.clear();
    net.requests.clear();
    net.failure = None;
    Ok(())
}

/// Leave the server, or stop being one. Offline afterwards; the remote
/// players leave the scene on its next update.
pub fn disconnect() {
    let mut net = net();
    match &mut net.role {
        Role::Client(client) => {
            client.transport.disconnect();
        }
        Role::Server(server) => {
            server.transport.disconnect_all(&mut server.server);
        }
        Role::Offline => {}
    }
    net.role = Role::Offline;
    net.events.clear();
    net.requests.clear();
    net.failure = None;
}

pub fn status() -> Status {
    let net = net();
    match &net.role {
        Role::Offline => match &net.failure {
            Some(reason) => Status::Failed(reason.clone()),
            None => Status::Offline,
        },
        Role::Server(server) => Status::Hosting {
            players: server.server.connected_clients(),
        },
        Role::Client(client) if client.client.is_connected() => Status::Connected,
        Role::Client(_) => Status::Connecting,
    }
}

static JOIN_ON_START: Mutex<Option<String>> = Mutex::new(None);

/// The game was started to join the server at `address` (the player's
/// `--connect`); its menu asks with `take_join_on_start`.
pub fn set_join_on_start(address: impl Into<String>) {
    *JOIN_ON_START.lock().unwrap_or_else(|e| e.into_inner()) = Some(address.into());
}

/// The server the game was started to join, once.
pub fn take_join_on_start() -> Option<String> {
    JOIN_ON_START.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// Requests on `channel` are judged by the game: they wait in
/// `take_requests` on the authority (the server, or the game itself when
/// offline), which makes events of the ones it accepts with `publish`.
/// Declare it on every peer, before requests come in. Without it a
/// request is an event as it is.
pub fn moderate(channel: &str) {
    let mut net = net();
    if !net.moderated.contains(channel) {
        net.moderated.insert(channel.to_string());
    }
}

/// Whether this process decides what happens to the world: the server, or
/// a game played offline. Clients only request.
pub fn is_authority() -> bool {
    !matches!(net().role, Role::Client(_))
}

/// The requests on moderated `channel` since the last call, with who made
/// them (`None`: this process). Only the authority gets any.
pub fn take_requests(channel: &str) -> Vec<(Option<ClientId>, Vec<u8>)> {
    net()
        .requests
        .get_mut(channel)
        .map(|queue| queue.drain(..).collect())
        .unwrap_or_default()
}

/// `take_requests`, deserialized; requests that do not read as `T` are
/// dropped (and logged).
pub fn take_request_values<T: DeserializeOwned>(channel: &str) -> Vec<(Option<ClientId>, T)> {
    take_requests(channel)
        .into_iter()
        .filter_map(|(from, data)| match bincode::deserialize(&data) {
            Ok(value) => Some((from, value)),
            Err(e) => {
                log::warn!("net: a request on '{channel}' does not read: {e}");
                None
            }
        })
        .collect()
}

/// On the authority: make `data` an event on `channel`, for every peer and
/// this one (and kept for later players). Elsewhere it does nothing.
pub fn publish(channel: &str, data: Vec<u8>) {
    let mut net = net();
    let net = &mut *net;
    match &mut net.role {
        Role::Offline => push_event(&mut net.events, channel, data),
        Role::Server(server) => server_event(server, &mut net.events, channel.to_string(), data),
        Role::Client(_) => log::warn!("net: a client cannot publish on '{channel}'"),
    }
}

/// `publish` with a value, serialized.
pub fn publish_value<T: Serialize>(channel: &str, value: &T) {
    publish(channel, encode(value));
}

/// On the server: what was kept of `channel` so far (for later players,
/// and in the saved world) becomes this one event, which must bring a
/// fresh world to the state the channel's events brought it to: a
/// snapshot. Keeps the history short. Nothing is sent; elsewhere it does
/// nothing.
pub fn replace_history(channel: &str, data: Vec<u8>) {
    if let Role::Server(server) = &mut net().role {
        server.log.retain(|(c, _)| c != channel);
        server.log.push((channel.to_string(), data));
    }
}

/// `replace_history` with a value, serialized.
pub fn replace_history_value<T: Serialize>(channel: &str, value: &T) {
    replace_history(channel, encode(value));
}

/// What a saved world holds: every channel's kept events, in order, and
/// where each player who was there was last.
#[derive(Serialize, serde::Deserialize)]
struct SavedWorld {
    version: u32,
    history: Vec<(String, Vec<u8>)>,
    players: Vec<(u64, PlayerState)>,
}

/// Before players were saved.
#[derive(serde::Deserialize)]
struct SavedWorldV1 {
    #[allow(dead_code)]
    version: u32,
    history: Vec<(String, Vec<u8>)>,
}

const SAVE_VERSION: u32 = 2;

/// On the server: write the world (what is kept of every channel) to
/// `path`, replacing it whole or not at all.
pub fn save_world(path: &Path) -> Result<(), String> {
    let (history, players) = match &net().role {
        Role::Server(server) => (
            server.log.clone(),
            server.last_seen.iter().map(|(k, s)| (*k, s.clone())).collect(),
        ),
        _ => return Err("only a server saves its world".into()),
    };
    let data = encode(&SavedWorld {
        version: SAVE_VERSION,
        history,
        players,
    });
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let partial = path.with_extension("partial");
    std::fs::write(&partial, data).map_err(|e| format!("{}: {e}", partial.display()))?;
    std::fs::rename(&partial, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// On the server: carry on the world saved at `path`. Its events are kept
/// for players who join, and applied here like new ones. Returns how many
/// there were.
pub fn load_world(path: &Path) -> Result<usize, String> {
    let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let unreadable = |e: bincode::Error| format!("{}: {e}", path.display());
    let version: u32 = bincode::deserialize(&data).map_err(unreadable)?;
    let saved = match version {
        1 => {
            let old: SavedWorldV1 = bincode::deserialize(&data).map_err(unreadable)?;
            SavedWorld {
                version,
                history: old.history,
                players: Vec::new(),
            }
        }
        SAVE_VERSION => bincode::deserialize(&data).map_err(unreadable)?,
        _ => {
            return Err(format!(
                "{}: saved by another version ({version})",
                path.display()
            ))
        }
    };
    let mut net = net();
    let net = &mut *net;
    let Role::Server(server) = &mut net.role else {
        return Err("only a server loads a world".into());
    };
    let count = saved.history.len();
    server.last_seen.extend(saved.players);
    for (channel, data) in saved.history {
        push_event(&mut net.events, &channel, data.clone());
        server.log.push((channel, data));
    }
    Ok(count)
}

/// The other players and where they are, as last heard (on the server:
/// every player).
pub fn players() -> Vec<(ClientId, PlayerState)> {
    let net = net();
    let players = match &net.role {
        Role::Offline => return Vec::new(),
        Role::Server(server) => &server.players,
        Role::Client(client) => &client.players,
    };
    let mut list: Vec<_> = players.iter().map(|(id, s)| (*id, s.clone())).collect();
    list.sort_by_key(|(id, _)| *id);
    list
}

/// Whether this process is the server.
pub fn is_server() -> bool {
    matches!(net().role, Role::Server(_))
}

/// Whether this process plays on a server (connected or connecting).
pub fn is_client() -> bool {
    matches!(net().role, Role::Client(_))
}

/// This player's number on the server, while connected.
pub fn local_id() -> Option<ClientId> {
    match &net().role {
        Role::Client(client) => Some(client.id),
        _ => None,
    }
}

/// Ask for a world event on `channel`. Offline (and on the server) it is
/// one at once; a client sends it to the server, which hands it to every
/// peer, this one too.
pub fn request(channel: &str, data: Vec<u8>) {
    let mut net = net();
    let net = &mut *net;
    let moderated = net.moderated.contains(channel);
    match &mut net.role {
        Role::Offline | Role::Server(_) if moderated => {
            push_request(&mut net.requests, channel, None, data)
        }
        Role::Offline => push_event(&mut net.events, channel, data),
        Role::Server(server) => {
            server_event(server, &mut net.events, channel.to_string(), data);
        }
        Role::Client(client) => {
            let message = encode(&ToServer::Request {
                channel: channel.to_string(),
                data,
            });
            client.client.send_message(DefaultChannel::ReliableOrdered, message);
        }
    }
}

/// `request` with a value, serialized.
pub fn request_value<T: Serialize>(channel: &str, value: &T) {
    request(channel, encode(value));
}

/// The events of `channel` that arrived, in order. Each is handed out once.
pub fn take_events(channel: &str) -> Vec<Vec<u8>> {
    net()
        .events
        .get_mut(channel)
        .map(|queue| queue.drain(..).collect())
        .unwrap_or_default()
}

/// `take_events`, deserialized; events that do not read as `T` are left
/// out (and logged).
pub fn take_values<T: DeserializeOwned>(channel: &str) -> Vec<T> {
    take_events(channel)
        .into_iter()
        .filter_map(|data| match bincode::deserialize(&data) {
            Ok(value) => Some(value),
            Err(e) => {
                log::warn!("net: an event on '{channel}' does not read: {e}");
                None
            }
        })
        .collect()
}

/// Events kept per channel at most; a channel nobody takes (no script of
/// this game listens to it here) does not grow without end.
const MAX_QUEUED: usize = 10_000;

fn push_request(
    requests: &mut HashMap<String, VecDeque<(Option<ClientId>, Vec<u8>)>>,
    channel: &str,
    from: Option<ClientId>,
    data: Vec<u8>,
) {
    let queue = requests.entry(channel.to_string()).or_default();
    if queue.len() >= MAX_QUEUED {
        queue.pop_front();
    }
    queue.push_back((from, data));
}

fn push_event(events: &mut HashMap<String, VecDeque<Vec<u8>>>, channel: &str, data: Vec<u8>) {
    let queue = events.entry(channel.to_string()).or_default();
    if queue.len() >= MAX_QUEUED {
        queue.pop_front();
    }
    queue.push_back(data);
}

/// On the server: `data` becomes an event, kept for later players, sent to
/// all and applied here.
fn server_event(
    server: &mut Server,
    events: &mut HashMap<String, VecDeque<Vec<u8>>>,
    channel: String,
    data: Vec<u8>,
) {
    let message = encode(&ToClient::Event {
        channel: channel.clone(),
        data: data.clone(),
    });
    server.server.broadcast_message(DefaultChannel::ReliableOrdered, message);
    push_event(events, &channel, data.clone());
    server.log.push((channel, data));
}

/// Send and receive, and keep the remote players in `scene` up to date.
/// Called by `Scene::update` every frame.
pub(crate) fn update(scene: &mut Scene, delta_time: f64) {
    let (offline, others, left, welcome) = {
        let mut net = net();
        let net = &mut *net;
        let elapsed = net
            .last_update
            .map_or(Duration::from_secs_f64(delta_time), |at| at.elapsed());
        net.last_update = Some(Instant::now());
        net.since_send += elapsed.as_secs_f32();
        let send = net.since_send >= 1.0 / SEND_RATE;
        if send {
            net.since_send = 0.0;
        }
        let own = if send && !matches!(net.role, Role::Offline) {
            players::local_state(scene)
        } else {
            None
        };
        let failure = match &mut net.role {
            Role::Offline => None,
            Role::Server(server) => {
                update_server(
                    server,
                    &mut net.events,
                    &mut net.requests,
                    &net.moderated,
                    elapsed,
                    send,
                );
                None
            }
            Role::Client(client) => update_client(client, &mut net.events, elapsed, own),
        };
        if let Some(reason) = failure {
            log::warn!("net: {reason}");
            net.role = Role::Offline;
            net.failure = Some(reason);
        }
        match &mut net.role {
            Role::Offline => (true, HashMap::new(), Vec::new(), None),
            Role::Server(server) => (
                false,
                server.players.clone(),
                std::mem::take(&mut server.left),
                None,
            ),
            Role::Client(client) => (
                false,
                client.players.clone(),
                std::mem::take(&mut client.left),
                client.welcome.take(),
            ),
        }
    };
    if let Some(welcome) = welcome {
        // The server's world instead of this one, then this player where
        // it belongs in it.
        if let Some(text) = welcome.scene {
            let loaded = ron::from_str::<SceneFile>(&text)
                .map_err(|e| e.to_string())
                .and_then(|file| Scene::from_file(&file).map_err(|e| e.to_string()));
            match loaded {
                Ok(server_scene) => scene.replace_with(server_scene),
                Err(e) => log::warn!("net: cannot play the server's scene: {e}"),
            }
        }
        players::place_local(scene, welcome.returning.as_ref(), welcome.index);
    }
    if offline {
        players::remove_all(scene);
    } else {
        players::sync(scene, &others, &left, delta_time as f32);
    }
}

fn update_server(
    server: &mut Server,
    events: &mut HashMap<String, VecDeque<Vec<u8>>>,
    requests: &mut HashMap<String, VecDeque<(Option<ClientId>, Vec<u8>)>>,
    moderated: &HashSet<String>,
    elapsed: Duration,
    send: bool,
) {
    server.server.update(elapsed);
    if let Err(e) = server.transport.update(elapsed, &mut server.server) {
        log::warn!("net: {e}");
    }
    while let Some(event) = server.server.get_event() {
        match event {
            ServerEvent::ClientConnected { client_id } => {
                let key = server
                    .transport
                    .user_data(client_id)
                    .map_or(client_id, |data| u64::from_le_bytes(data[..8].try_into().unwrap()));
                let returning = server.last_seen.get(&key).cloned();
                log::info!(
                    "net: player {client_id} joined{}",
                    if returning.is_some() { " (back again)" } else { "" }
                );
                server.keys.insert(client_id, key);
                let welcome = encode(&ToClient::Welcome {
                    scene: server.scene.clone(),
                    returning,
                    index: server.joined,
                });
                server.joined += 1;
                server
                    .server
                    .send_message(client_id, DefaultChannel::ReliableOrdered, welcome);
                // The world as it is: every event so far.
                for (channel, data) in &server.log {
                    let message = encode(&ToClient::Event {
                        channel: channel.clone(),
                        data: data.clone(),
                    });
                    server
                        .server
                        .send_message(client_id, DefaultChannel::ReliableOrdered, message);
                }
            }
            ServerEvent::ClientDisconnected { client_id, reason } => {
                log::info!("net: player {client_id} left ({reason})");
                server.players.remove(&client_id);
                server.keys.remove(&client_id);
                server.left.push(client_id);
                server
                    .server
                    .broadcast_message(DefaultChannel::ReliableOrdered, encode(&ToClient::Left(client_id)));
            }
        }
    }
    for client_id in server.server.clients_id() {
        for lane in [DefaultChannel::ReliableOrdered, DefaultChannel::Unreliable] {
            let lane = u8::from(lane);
            while let Some(message) = server.server.receive_message(client_id, lane) {
                match bincode::deserialize::<ToServer>(&message) {
                    Ok(ToServer::Request { channel, data }) if moderated.contains(&channel) => {
                        push_request(requests, &channel, Some(client_id), data)
                    }
                    Ok(ToServer::Request { channel, data }) => {
                        server_event(server, events, channel, data)
                    }
                    Ok(ToServer::State(state)) => {
                        if let Some(key) = server.keys.get(&client_id) {
                            server.last_seen.insert(*key, state.clone());
                        }
                        server.players.insert(client_id, state);
                    }
                    Err(e) => log::warn!("net: bad message from {client_id}: {e}"),
                }
            }
        }
    }
    if send && !server.players.is_empty() {
        let all: Vec<(ClientId, PlayerState)> =
            server.players.iter().map(|(id, s)| (*id, s.clone())).collect();
        server
            .server
            .broadcast_message(DefaultChannel::Unreliable, encode(&ToClient::Players(all)));
    }
    server.transport.send_packets(&mut server.server);
}

/// Returns why the connection ended, if it did.
fn update_client(
    client: &mut Client,
    events: &mut HashMap<String, VecDeque<Vec<u8>>>,
    elapsed: Duration,
    own: Option<PlayerState>,
) -> Option<String> {
    client.client.update(elapsed);
    if let Err(e) = client.transport.update(elapsed, &mut client.client) {
        return Some(format!("connection lost: {e}"));
    }
    if client.client.is_disconnected() {
        let reason = client
            .client
            .disconnect_reason()
            .map_or("disconnected".to_string(), |r| r.to_string());
        return Some(reason);
    }
    if !client.client.is_connected() {
        if client.started.elapsed() > CONNECT_TIMEOUT {
            client.transport.disconnect();
            return Some("the server did not answer".into());
        }
        let _ = client.transport.send_packets(&mut client.client);
        return None;
    }
    for lane in [DefaultChannel::ReliableOrdered, DefaultChannel::Unreliable] {
        let lane = u8::from(lane);
        while let Some(message) = client.client.receive_message(lane) {
            match bincode::deserialize::<ToClient>(&message) {
                Ok(ToClient::Welcome {
                    scene,
                    returning,
                    index,
                }) => {
                    client.welcome = Some(Welcome {
                        scene,
                        returning,
                        index,
                    });
                }
                Ok(ToClient::Event { channel, data }) => push_event(events, &channel, data),
                Ok(ToClient::Players(all)) => {
                    client.players = all.into_iter().filter(|(id, _)| *id != client.id).collect();
                }
                Ok(ToClient::Left(id)) => {
                    client.players.remove(&id);
                    client.left.push(id);
                }
                Err(e) => log::warn!("net: bad message from the server: {e}"),
            }
        }
    }
    if let Some(state) = own {
        client
            .client
            .send_message(DefaultChannel::Unreliable, encode(&ToServer::State(state)));
    }
    if let Err(e) = client.transport.send_packets(&mut client.client) {
        return Some(format!("connection lost: {e}"));
    }
    None
}

/// For a dedicated server: the scene has no player of its own. Takes the
/// scene's players out (remembering what one looks like, for the remote
/// players).
pub fn make_dedicated(scene: &mut Scene) {
    players::remove_local_players(scene);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One test for all of it: the state is global.
    #[test]
    fn requests_events_and_saved_worlds() {
        offline_requests_are_events_at_once();

        // Moderated: a request waits for the authority, which publishes.
        moderate("judged");
        request_value("judged", &7u8);
        assert!(take_events("judged").is_empty());
        let requests: Vec<(Option<ClientId>, u8)> = take_request_values("judged");
        assert_eq!(requests, vec![(None, 7)]);
        publish_value("judged", &8u8);
        assert_eq!(take_values::<u8>("judged"), vec![8]);

        // A server keeps what is published, a snapshot replaces it, and a
        // saved world comes back as events.
        host(0).unwrap();
        publish_value("kept", &1u8);
        publish_value("kept", &2u8);
        publish_value("other", &3u8);
        replace_history_value("kept", &12u8);
        let _ = take_events("kept");
        let _ = take_events("other");
        let path = std::env::temp_dir().join(format!("ferrite-world-{}.sav", std::process::id()));
        save_world(&path).unwrap();
        disconnect();
        host(0).unwrap();
        assert_eq!(load_world(&path).unwrap(), 2);
        assert_eq!(take_values::<u8>("kept"), vec![12]);
        assert_eq!(take_values::<u8>("other"), vec![3]);
        disconnect();
        let _ = std::fs::remove_file(&path);
    }

    fn offline_requests_are_events_at_once() {
        // Shares the global state; nothing else in the tests goes online.
        request_value("test-channel", &(1u32, "a".to_string()));
        request_value("test-channel", &(2u32, "b".to_string()));
        let events: Vec<(u32, String)> = take_values("test-channel");
        assert_eq!(events, vec![(1, "a".into()), (2, "b".into())]);
        assert!(take_events("test-channel").is_empty());
        assert_eq!(status(), Status::Offline);
    }
}
