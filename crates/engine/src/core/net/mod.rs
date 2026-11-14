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
//! `Scene::update` runs all of this; a game only requests and applies its
//! own events, and offers a way to `host` or `connect`.

mod players;
mod protocol;

use std::{
    collections::{HashMap, VecDeque},
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

use super::scene::Scene;
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
}

struct Client {
    client: RenetClient,
    transport: NetcodeClientTransport,
    id: ClientId,
    started: Instant,
    /// Latest state of the other players.
    players: HashMap<ClientId, PlayerState>,
    left: Vec<ClientId>,
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
    /// Why the last connection ended, until something else happens.
    failure: Option<String>,
    last_update: Option<Instant>,
    since_send: f32,
}

static NET: LazyLock<Mutex<Net>> = LazyLock::new(|| {
    Mutex::new(Net {
        role: Role::Offline,
        events: HashMap::new(),
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
    }));
    net.events.clear();
    net.failure = None;
    Ok(())
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
    let authentication = ClientAuthentication::Unsecure {
        protocol_id: PROTOCOL_ID,
        client_id: id,
        server_addr,
        user_data: None,
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
    }));
    net.events.clear();
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
    match &mut net.role {
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
    let (offline, others, left) = {
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
                update_server(server, &mut net.events, elapsed, send);
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
            Role::Offline => (true, HashMap::new(), Vec::new()),
            Role::Server(server) => (
                false,
                server.players.clone(),
                std::mem::take(&mut server.left),
            ),
            Role::Client(client) => (
                false,
                client.players.clone(),
                std::mem::take(&mut client.left),
            ),
        }
    };
    if offline {
        players::remove_all(scene);
    } else {
        players::sync(scene, &others, &left, delta_time as f32);
    }
}

fn update_server(
    server: &mut Server,
    events: &mut HashMap<String, VecDeque<Vec<u8>>>,
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
                log::info!("net: player {client_id} joined");
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
                    Ok(ToServer::Request { channel, data }) => {
                        server_event(server, events, channel, data)
                    }
                    Ok(ToServer::State(state)) => {
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

    #[test]
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
