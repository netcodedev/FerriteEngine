//! What server and clients send each other.

use serde::{Deserialize, Serialize};

use super::{ClientId, PlayerState};

#[derive(Serialize, Deserialize)]
pub enum ToServer {
    /// A world event this client asks for.
    Request { channel: String, data: Vec<u8> },
    /// Where this client's player is (unreliable, many times a second).
    State(PlayerState),
}

#[derive(Serialize, Deserialize)]
pub enum ToClient {
    /// A world event, in the order the server made them.
    Event { channel: String, data: Vec<u8> },
    /// Every player (unreliable, many times a second).
    Players(Vec<(ClientId, PlayerState)>),
    /// A player left.
    Left(ClientId),
}
