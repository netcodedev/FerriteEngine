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
    fs::File,
    path::{Path, PathBuf},
    net::{SocketAddr, ToSocketAddrs, UdpSocket},
    sync::{
        mpsc::{self, Receiver, TryRecvError},
        LazyLock, Mutex, MutexGuard,
    },
    time::{Duration, Instant, SystemTime},
};

use renet::{ChannelConfig, ConnectionConfig, DefaultChannel, RenetClient, RenetServer, ServerEvent};
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

/// Most bytes the reliable channels hold for one peer, sent but not yet
/// acknowledged (renet disconnects a peer whose channel overflows). The
/// server never fills it past this (see `Server::outbox`), and a scene sent
/// on joining has to fit.
const RELIABLE_MEMORY: usize = 32 * 1024 * 1024;
/// Bytes of queued events (the history, on joining) the server hands each
/// client's reliable channel per update at most.
const OUTBOX_BYTES_PER_TICK: usize = 256 * 1024;
/// Largest request a client may send; bigger ones are dropped.
const MAX_REQUEST_BYTES: usize = 64 * 1024;
/// Requests and their bytes one client may send per second; more are
/// dropped (a client that floods the server with requests would grow every
/// channel's history without end).
const MAX_REQUESTS_PER_SECOND: u32 = 240;
const MAX_REQUEST_BYTES_PER_SECOND: usize = 1024 * 1024;
/// Size of the kept history (over all channels) from which the server
/// warns that it should be replaced by snapshots (`replace_history`); again
/// at every doubling.
const HISTORY_WARNING_BYTES: usize = 8 * 1024 * 1024;

/// renet's channels, with room for `RELIABLE_MEMORY` in the reliable ones.
/// Only how much each side keeps changes, not what goes over the wire.
fn connection_config() -> ConnectionConfig {
    let channels: Vec<ChannelConfig> = DefaultChannel::config()
        .into_iter()
        .map(|mut channel| {
            if channel.channel_id != u8::from(DefaultChannel::Unreliable) {
                channel.max_memory_usage_bytes = RELIABLE_MEMORY;
            }
            channel
        })
        .collect();
    ConnectionConfig {
        server_channels_config: channels.clone(),
        client_channels_config: channels,
        ..ConnectionConfig::default()
    }
}

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
    /// Reliable messages waiting for a client's channel to have room, in
    /// order: the history a client gets on joining is handed over a bit
    /// per update instead of all at once, and events made meanwhile wait
    /// behind it.
    outbox: HashMap<ClientId, VecDeque<Vec<u8>>>,
    /// What each client requested in the current second.
    request_budget: HashMap<ClientId, RequestBudget>,
    /// Bytes in `log`, and from which size the next warning comes.
    log_bytes: usize,
    log_warning_at: usize,
}

/// Requests of one client in the second since `since`.
struct RequestBudget {
    since: Instant,
    count: u32,
    bytes: usize,
    /// Whether dropping was logged this second.
    warned: bool,
}

impl RequestBudget {
    fn new() -> Self {
        Self {
            since: Instant::now(),
            count: 0,
            bytes: 0,
            warned: false,
        }
    }

    /// Whether a request of `bytes` may still come this second (and count
    /// it if so).
    fn allow(&mut self, bytes: usize) -> bool {
        if self.since.elapsed() >= Duration::from_secs(1) {
            *self = Self::new();
        }
        if bytes > MAX_REQUEST_BYTES
            || self.count >= MAX_REQUESTS_PER_SECOND
            || self.bytes + bytes > MAX_REQUEST_BYTES_PER_SECOND
        {
            return false;
        }
        self.count += 1;
        self.bytes += bytes;
        true
    }
}

struct Client {
    client: RenetClient,
    link: Link,
    id: ClientId,
    /// When the connection was made (after the lookup), for the timeout.
    started: Instant,
    /// Latest state of the other players.
    players: HashMap<ClientId, PlayerState>,
    left: Vec<ClientId>,
    /// Arrived, not carried out yet.
    welcome: Option<Welcome>,
}

/// How a client reaches its server.
enum Link {
    /// The server's name is being looked up on another thread (a name
    /// server can take seconds to answer); the connection is made on the
    /// first update after it is known.
    Lookup {
        address: Receiver<Result<SocketAddr, String>>,
        socket: Option<UdpSocket>,
        user_data: [u8; renet_netcode::NETCODE_USER_DATA_BYTES],
    },
    Ready(NetcodeClientTransport),
}

impl Client {
    /// Make the connection once the lookup has the server's address.
    /// `Ok(false)` while it is still looked up.
    fn finish_lookup(&mut self) -> Result<bool, String> {
        let Link::Lookup {
            address,
            socket,
            user_data,
        } = &mut self.link
        else {
            return Ok(true);
        };
        let server_addr = match address.try_recv() {
            Ok(found) => found?,
            Err(TryRecvError::Empty) => return Ok(false),
            Err(TryRecvError::Disconnected) => return Err("the address lookup failed".into()),
        };
        let socket = socket.take().ok_or("the address lookup failed")?;
        let transport = client_transport(self.id, *user_data, server_addr, socket)?;
        self.link = Link::Ready(transport);
        self.started = Instant::now();
        Ok(true)
    }
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
        server: RenetServer::new(connection_config()),
        transport,
        log: Vec::new(),
        players: HashMap::new(),
        left: Vec::new(),
        scene: None,
        keys: HashMap::new(),
        last_seen: HashMap::new(),
        joined: 0,
        outbox: HashMap::new(),
        request_budget: HashMap::new(),
        log_bytes: 0,
        log_warning_at: HISTORY_WARNING_BYTES,
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
///
/// The file is locked while the game runs: a second game of the same
/// project on this machine (the editor's play mode and a player started
/// with `--connect`, say) is another player, kept in `saves/player-2.id`,
/// and so on.
fn player_key() -> u64 {
    static HELD: Mutex<Option<(PathBuf, u64, File)>> = Mutex::new(None);
    let saves = Path::new("saves");
    let _ = std::fs::create_dir_all(saves);
    let dir = saves.canonicalize().unwrap_or_else(|_| saves.to_path_buf());
    let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((held_dir, key, _)) = &*held {
        if *held_dir == dir {
            return *key;
        }
    }
    match claim_player_key(&dir) {
        Some((key, file)) => {
            *held = Some((dir, key, file));
            key
        }
        None => {
            log::warn!("net: cannot keep who this player is in {}", dir.display());
            new_player_key()
        }
    }
}

fn new_player_key() -> u64 {
    now().as_nanos() as u64 ^ (std::process::id() as u64).rotate_left(40)
}

/// The first player id file in `dir` no other game holds, locked, with
/// the key in it (made if it is new).
fn claim_player_key(dir: &Path) -> Option<(u64, File)> {
    use std::io::{Read, Seek, Write};
    for slot in 1..=MAX_PLAYERS {
        let name = match slot {
            1 => "player.id".to_string(),
            n => format!("player-{n}.id"),
        };
        let path = dir.join(name);
        let Ok(mut file) = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
        else {
            continue;
        };
        if file.try_lock().is_err() {
            continue;
        }
        let mut text = String::new();
        let _ = file.read_to_string(&mut text);
        if let Ok(key) = u64::from_str_radix(text.trim(), 16) {
            return Some((key, file));
        }
        let key = new_player_key();
        let written = file
            .set_len(0)
            .and_then(|()| file.rewind())
            .and_then(|()| write!(file, "{key:016x}\n"));
        if let Err(e) = written {
            log::warn!("net: cannot keep {}: {e}", path.display());
        }
        return Some((key, file));
    }
    None
}

/// The IPv4 address of `with_port` ("host:port"); `address` names it in
/// errors. Asks a name server unless the host is an IP address.
fn resolve(address: &str, with_port: &str) -> Result<SocketAddr, String> {
    with_port
        .to_socket_addrs()
        .map_err(|e| format!("{address}: {e}"))?
        .find(SocketAddr::is_ipv4)
        .ok_or_else(|| format!("{address}: no IPv4 address"))
}

fn client_transport(
    id: ClientId,
    user_data: [u8; renet_netcode::NETCODE_USER_DATA_BYTES],
    server_addr: SocketAddr,
    socket: UdpSocket,
) -> Result<NetcodeClientTransport, String> {
    let authentication = ClientAuthentication::Unsecure {
        protocol_id: PROTOCOL_ID,
        client_id: id,
        server_addr,
        user_data: Some(user_data),
    };
    NetcodeClientTransport::new(now(), authentication, socket).map_err(|e| e.to_string())
}

/// Connect to the server at `address` ("host" or "host:port"). Returns at
/// once; `status` tells when it is connected. A host name is looked up
/// meanwhile (`status` is `Connecting`, and `Failed` if it is unknown).
pub fn connect(address: &str) -> Result<(), String> {
    let address = address.trim();
    let with_port = if address.contains(':') {
        address.to_string()
    } else {
        format!("{address}:{DEFAULT_PORT}")
    };
    // Numbers need no name server: those, and what is not an address at
    // all, are answered at once. A host name is looked up on another thread.
    let named = with_port.parse::<SocketAddr>().is_err()
        && with_port
            .rsplit_once(':')
            .is_some_and(|(_, port)| port.parse::<u16>().is_ok());
    let looked_up = if named {
        None
    } else {
        Some(resolve(address, &with_port)?)
    };
    let socket = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
    let id = now().as_nanos() as u64 ^ (std::process::id() as u64) << 32;
    // Who this player is travels with the connection request.
    let mut user_data = [0u8; renet_netcode::NETCODE_USER_DATA_BYTES];
    user_data[..8].copy_from_slice(&player_key().to_le_bytes());
    let link = match looked_up {
        Some(server_addr) => Link::Ready(client_transport(id, user_data, server_addr, socket)?),
        None => {
            let (found, lookup) = mpsc::channel();
            let name = address.to_string();
            std::thread::Builder::new()
                .name("net lookup".into())
                .spawn(move || {
                    // Nobody listens any more once connecting was given up.
                    let _ = found.send(resolve(&name, &with_port));
                })
                .map_err(|e| e.to_string())?;
            Link::Lookup {
                address: lookup,
                socket: Some(socket),
                user_data,
            }
        }
    };
    let mut net = net();
    net.role = Role::Client(Box::new(Client {
        client: RenetClient::new(connection_config()),
        link,
        id,
        started: Instant::now(),
        players: HashMap::new(),
        left: Vec::new(),
        welcome: None,
    }));
    net.events.clear();
    net.requests.clear();
    net.failure = None;
    // What the server's players look like is taken from its scene.
    players::forget_template();
    Ok(())
}

/// Leave the server, or stop being one. Offline afterwards; the remote
/// players leave the scene on its next update.
pub fn disconnect() {
    let mut net = net();
    match &mut net.role {
        Role::Client(client) => {
            if let Link::Ready(transport) = &mut client.link {
                transport.disconnect();
            }
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
    // The next session may be in another scene, or project. (Not on `host`:
    // a dedicated server takes it from its scene just before, see
    // `make_dedicated`.)
    players::forget_template();
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
        server.log_bytes = server.log.iter().map(|(c, d)| c.len() + d.len()).sum();
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
        // All of it, however long: the scripts have not run yet, and the
        // world they build has to be the one players get.
        net.events.entry(channel.clone()).or_default().push_back(data.clone());
        keep_in_log(server, channel, data);
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
        static WARNED: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Default::default);
        if WARNED.lock().unwrap_or_else(|e| e.into_inner()).insert(channel.to_string()) {
            log::warn!(
                "net: nobody takes the events of '{channel}'; dropping the oldest beyond \
                 {MAX_QUEUED}"
            );
        }
    }
    queue.push_back(data);
}

/// On the server: keep an event for later players (and the saved world).
fn keep_in_log(server: &mut Server, channel: String, data: Vec<u8>) {
    server.log_bytes += channel.len() + data.len();
    server.log.push((channel, data));
    if server.log_bytes >= server.log_warning_at {
        log::warn!(
            "net: the world's history holds {} MiB; every player who joins is sent all of it. \
             Replace channels' histories with snapshots (replace_history).",
            server.log_bytes / (1024 * 1024)
        );
        server.log_warning_at = server.log_warning_at.saturating_mul(2);
    }
}

/// On the server: send `message` to `client_id` reliably and in order:
/// now if its channel has room and nothing waits before it, else after
/// what waits (see `flush_outbox`).
fn send_reliable(server: &mut Server, client_id: ClientId, message: Vec<u8>) {
    let outbox = server.outbox.entry(client_id).or_default();
    if outbox.is_empty()
        && server
            .server
            .can_send_message(client_id, DefaultChannel::ReliableOrdered, message.len())
    {
        server
            .server
            .send_message(client_id, DefaultChannel::ReliableOrdered, message);
    } else {
        outbox.push_back(message);
    }
}

/// `send_reliable` to every connected client.
fn broadcast_reliable(server: &mut Server, message: Vec<u8>) {
    for client_id in server.server.clients_id() {
        send_reliable(server, client_id, message.clone());
    }
}

/// On the server: hand each client's channel what waits for it, as much as
/// it has room for, up to `OUTBOX_BYTES_PER_TICK`.
fn flush_outbox(renet: &mut RenetServer, outbox: &mut HashMap<ClientId, VecDeque<Vec<u8>>>) {
    outbox.retain(|&client_id, queue| {
        if !renet.is_connected(client_id) {
            return false;
        }
        let mut sent = 0;
        while let Some(message) = queue.front() {
            if message.len() > RELIABLE_MEMORY {
                log::error!(
                    "net: a message of {} MiB cannot be sent to player {client_id}; the most is \
                     {} MiB",
                    message.len() / (1024 * 1024),
                    RELIABLE_MEMORY / (1024 * 1024)
                );
                queue.pop_front();
                continue;
            }
            // One message at least, however big, when there is room.
            if (sent > 0 && sent + message.len() > OUTBOX_BYTES_PER_TICK)
                || !renet.can_send_message(client_id, DefaultChannel::ReliableOrdered, message.len())
            {
                break;
            }
            let message = queue.pop_front().expect("checked");
            sent += message.len();
            renet.send_message(client_id, DefaultChannel::ReliableOrdered, message);
        }
        !queue.is_empty()
    });
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
    broadcast_reliable(server, message);
    push_event(events, &channel, data.clone());
    keep_in_log(server, channel, data);
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
        // Other players look like the player of the scene played now.
        players::refresh_template(scene);
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
                send_reliable(server, client_id, welcome);
                // The world as it is: every event so far, handed over a bit
                // per update (see `flush_outbox`), events made meanwhile
                // behind it.
                let history: Vec<Vec<u8>> = server
                    .log
                    .iter()
                    .map(|(channel, data)| {
                        encode(&ToClient::Event {
                            channel: channel.clone(),
                            data: data.clone(),
                        })
                    })
                    .collect();
                server.outbox.entry(client_id).or_default().extend(history);
            }
            ServerEvent::ClientDisconnected { client_id, reason } => {
                log::info!("net: player {client_id} left ({reason})");
                server.players.remove(&client_id);
                server.keys.remove(&client_id);
                server.outbox.remove(&client_id);
                server.request_budget.remove(&client_id);
                server.left.push(client_id);
                broadcast_reliable(server, encode(&ToClient::Left(client_id)));
            }
        }
    }
    for client_id in server.server.clients_id() {
        for lane in [DefaultChannel::ReliableOrdered, DefaultChannel::Unreliable] {
            let lane = u8::from(lane);
            while let Some(message) = server.server.receive_message(client_id, lane) {
                let parsed = bincode::deserialize::<ToServer>(&message);
                if let Ok(ToServer::Request { .. }) = &parsed {
                    let budget = server
                        .request_budget
                        .entry(client_id)
                        .or_insert_with(RequestBudget::new);
                    if !budget.allow(message.len()) {
                        if !budget.warned {
                            budget.warned = true;
                            log::warn!(
                                "net: player {client_id} sends too many or too big requests; \
                                 dropping them"
                            );
                        }
                        continue;
                    }
                }
                match parsed {
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
    flush_outbox(&mut server.server, &mut server.outbox);
    server.transport.send_packets(&mut server.server);
}

/// Returns why the connection ended, if it did.
fn update_client(
    client: &mut Client,
    events: &mut HashMap<String, VecDeque<Vec<u8>>>,
    elapsed: Duration,
    own: Option<PlayerState>,
) -> Option<String> {
    match client.finish_lookup() {
        Ok(true) => {}
        Ok(false) => return None,
        Err(reason) => return Some(reason),
    }
    let Link::Ready(transport) = &mut client.link else {
        return None;
    };
    client.client.update(elapsed);
    if let Err(e) = transport.update(elapsed, &mut client.client) {
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
            transport.disconnect();
            return Some("the server did not answer".into());
        }
        let _ = transport.send_packets(&mut client.client);
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
    if let Err(e) = transport.send_packets(&mut client.client) {
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

        // A long history comes back whole, not cut to what a queue keeps.
        for i in 0..MAX_QUEUED as u32 + 5 {
            publish_value("long", &i);
        }
        let _ = take_events("long");
        save_world(&path).unwrap();
        disconnect();
        host(0).unwrap();
        load_world(&path).unwrap();
        assert_eq!(take_events("long").len(), MAX_QUEUED + 5);
        disconnect();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_long_history_is_handed_to_a_joining_client_without_overflowing_its_channel() {
        let mut renet = RenetServer::new(connection_config());
        renet.add_connection(1);
        // More than the channel holds unacknowledged (nothing is
        // acknowledged here: no client answers).
        let mut outbox = HashMap::new();
        let count = RELIABLE_MEMORY / (64 * 1024) + 100;
        let history: VecDeque<Vec<u8>> = (0..count).map(|_| vec![7u8; 64 * 1024]).collect();
        outbox.insert(1, history);
        for ticks in 0..300 {
            flush_outbox(&mut renet, &mut outbox);
            renet.update(Duration::from_millis(16));
            assert!(renet.is_connected(1), "dropped after {ticks} updates");
        }
        // What did not fit waits for room.
        let waiting = outbox.get(&1).map_or(0, VecDeque::len);
        assert!(waiting >= 100, "{waiting}");
    }

    #[test]
    fn a_client_may_request_only_so_much_per_second() {
        let mut budget = RequestBudget::new();
        assert!(!budget.allow(MAX_REQUEST_BYTES + 1));
        for _ in 0..MAX_REQUESTS_PER_SECOND {
            assert!(budget.allow(16));
        }
        assert!(!budget.allow(16));
        let mut budget = RequestBudget::new();
        let mut bytes = 0;
        while budget.allow(MAX_REQUEST_BYTES) {
            bytes += MAX_REQUEST_BYTES;
        }
        assert!(bytes <= MAX_REQUEST_BYTES_PER_SECOND);
        // A second later, again.
        budget.since -= Duration::from_secs(1);
        assert!(budget.allow(16));
    }

    #[test]
    fn two_games_of_one_project_are_two_players() {
        let dir = std::env::temp_dir().join(format!("ferrite-player-key-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (first, held) = claim_player_key(&dir).unwrap();
        let (second, other) = claim_player_key(&dir).unwrap();
        assert_ne!(first, second);
        assert!(dir.join("player-2.id").is_file());
        drop(held);
        drop(other);
        // The same again once the games ended.
        assert_eq!(claim_player_key(&dir).unwrap().0, first);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A client still looking up its server, with the lookup's answer
    /// sent through the returned sender.
    fn looking_up() -> (Client, mpsc::Sender<Result<SocketAddr, String>>) {
        let (found, address) = mpsc::channel();
        let client = Client {
            client: RenetClient::new(connection_config()),
            link: Link::Lookup {
                address,
                socket: Some(UdpSocket::bind("127.0.0.1:0").unwrap()),
                user_data: [0; renet_netcode::NETCODE_USER_DATA_BYTES],
            },
            id: 1,
            started: Instant::now(),
            players: HashMap::new(),
            left: Vec::new(),
            welcome: None,
        };
        (client, found)
    }

    #[test]
    fn a_client_waits_for_the_lookup_without_blocking_and_then_connects() {
        let (mut client, found) = looking_up();
        let mut events = HashMap::new();
        // A slow name server.
        let answer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            found.send(Ok(SocketAddr::from(([127, 0, 0, 1], 9)))).unwrap();
        });
        let start = Instant::now();
        assert_eq!(update_client(&mut client, &mut events, Duration::ZERO, None), None);
        assert!(start.elapsed() < Duration::from_millis(100));
        assert!(matches!(client.link, Link::Lookup { .. }));
        answer.join().unwrap();
        assert_eq!(update_client(&mut client, &mut events, Duration::ZERO, None), None);
        assert!(matches!(client.link, Link::Ready(_)));
        // The connection's time starts once it is made.
        assert!(client.started >= start + Duration::from_millis(200));
    }

    #[test]
    fn a_failed_lookup_ends_connecting_with_its_reason() {
        let (mut client, found) = looking_up();
        found.send(Err("nowhere: unknown".into())).unwrap();
        let failure = update_client(&mut client, &mut HashMap::new(), Duration::ZERO, None);
        assert_eq!(failure.as_deref(), Some("nowhere: unknown"));

        let (mut client, found) = looking_up();
        drop(found);
        let failure = update_client(&mut client, &mut HashMap::new(), Duration::ZERO, None);
        assert!(failure.is_some());
    }

    #[test]
    fn ip_addresses_are_resolved_without_a_name_server() {
        assert_eq!(
            resolve("10.0.0.2", "10.0.0.2:7777"),
            Ok(SocketAddr::from(([10, 0, 0, 2], 7777)))
        );
        assert!(resolve("host:x", "host:x").is_err());
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
