use crate::state::Player;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// An incoming WebSocket frame sent from the client.
pub enum ClientEvent {
    Message(ClientMessage),
    Close,
    Skip,
}

/// A message sent from the client to the server over WebSocket.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    CreateRoom {
        player_name: String,
    },
    JoinRoom {
        room_code: String,
        player_name: String,
    },
    StartGame,
}

/// A message sent from the server to the client over WebSocket.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    RoomCreated {
        room_code: String,
        my_player_id: Uuid,
        players: Vec<Player>,
    },
    RoomJoined {
        room_code: String,
        host_id: Uuid,
        my_player_id: Uuid,
        players: Vec<Player>,
    },
    PlayerJoined {
        player: Player,
        players: Vec<Player>,
    },
    PlayerDisconnected {
        player_id: Uuid,
        players: Vec<Player>,
    },
    GameStarted {
        starts_at_unix_ms: u64,
    },
}
