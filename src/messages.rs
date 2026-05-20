use crate::state::Player;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
}

#[derive(Debug, Serialize)]
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
}
