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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::net::PlayerState;

    fn player_state() -> PlayerState {
        PlayerState {
            position: [1.0, 2.0, 3.0],
            rotation: [1.0, 0.0, 0.0, 0.0],
            inputs: vec![("speed".to_string(), 0.5)],
        }
    }

    fn roundtrip_to_server(msg: &ToServer) -> ToServer {
        let bytes = bincode::serialize(msg).expect("serialize");
        bincode::deserialize(&bytes).expect("deserialize")
    }

    fn roundtrip_to_client(msg: &ToClient) -> ToClient {
        let bytes = bincode::serialize(msg).expect("serialize");
        bincode::deserialize(&bytes).expect("deserialize")
    }

    #[test]
    fn a_request_roundtrips_through_bincode() {
        let msg = ToServer::Request {
            channel: "doors".to_string(),
            data: vec![1, 2, 3, 4, 5],
        };
        match roundtrip_to_server(&msg) {
            ToServer::Request { channel, data } => {
                assert_eq!(channel, "doors");
                assert_eq!(data, vec![1, 2, 3, 4, 5]);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn a_player_state_update_roundtrips() {
        let msg = ToServer::State(player_state());
        match roundtrip_to_server(&msg) {
            ToServer::State(state) => {
                assert_eq!(state.position, [1.0, 2.0, 3.0]);
                assert_eq!(state.inputs, vec![("speed".to_string(), 0.5)]);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn a_welcome_roundtrips_with_and_without_a_scene() {
        for scene in [None, Some("# a scene".to_string())] {
            let msg = ToClient::Welcome {
                scene: scene.clone(),
                returning: None,
                index: 7,
            };
            match roundtrip_to_client(&msg) {
                ToClient::Welcome {
                    scene: got_scene,
                    returning,
                    index,
                } => {
                    assert_eq!(got_scene, scene);
                    assert!(returning.is_none());
                    assert_eq!(index, 7);
                }
                _ => panic!("wrong variant"),
            }
        }
    }

    #[test]
    fn an_event_roundtrips_through_bincode() {
        let msg = ToClient::Event {
            channel: "doors".to_string(),
            data: vec![0xDE, 0xAD, 0xBE, 0xEF],
        };
        match roundtrip_to_client(&msg) {
            ToClient::Event { channel, data } => {
                assert_eq!(channel, "doors");
                assert_eq!(data, vec![0xDE, 0xAD, 0xBE, 0xEF]);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn a_left_message_carries_the_client_id() {
        let msg = ToClient::Left(123456);
        match roundtrip_to_client(&msg) {
            ToClient::Left(id) => assert_eq!(id, 123456),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn a_players_update_roundtrips_every_player() {
        let msg = ToClient::Players(vec![(1, player_state()), (2, player_state())]);
        match roundtrip_to_client(&msg) {
            ToClient::Players(list) => {
                assert_eq!(list.len(), 2);
                assert_eq!(list[0].0, 1);
                assert_eq!(list[1].0, 2);
            }
            _ => panic!("wrong variant"),
        }
    }
}
