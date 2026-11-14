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
    /// First thing after connecting: the server's scene (RON of a scene
    /// file) for the client to play in instead of its own, and where its
    /// player was when it last left, if it was here before; else the how
    /// many-th player it is, to stand it apart from the others.
    Welcome {
        scene: Option<String>,
        returning: Option<PlayerState>,
        index: u32,
    },
    /// A world event, in the order the server made them.
    Event { channel: String, data: Vec<u8> },
    /// Every player (unreliable, many times a second).
    Players(Vec<(ClientId, PlayerState)>),
    /// A player left.
    Left(ClientId),
}
