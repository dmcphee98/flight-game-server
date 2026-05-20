mod messages;
mod state;

use axum::extract::ws::{Message, WebSocket};
use axum::{
    Router,
    extract::{State, WebSocketUpgrade},
    routing::get,
};
use messages::{ClientMessage, ServerMessage};
use state::{Player, Room, ServerState, SharedServerState};
use std::{net::SocketAddr, sync::Arc};

#[tokio::main]
async fn main() {
    let state: SharedServerState = Arc::new(ServerState::default());

    let app = Router::new()
        .route("/", get(root_handler))
        .route("/ws", get(websocket_handler))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 8080));
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    println!("Server running at {}", addr);
    axum::serve(listener, app).await.unwrap();
}

async fn root_handler() -> &'static str {
    "Hello, world!"
}

async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<SharedServerState>,
) -> impl axum::response::IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

/// Handles an individual WebSocket connection for its lifetime.
/// Exits when the client disconnects or a send error occurs.
async fn handle_socket(mut socket: WebSocket, state: SharedServerState) {
    while let Some(Ok(msg)) = socket.recv().await {
        match msg {
            Message::Text(text) => {
                let Ok(client_msg) = serde_json::from_str::<ClientMessage>(&text) else {
                    continue;
                };
                if let Some(response) = handle_client_message(client_msg, &state).await {
                    let json = serde_json::to_string(&response).unwrap();
                    if socket.send(Message::Text(json.into())).await.is_err() {
                        break;
                    }
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
}

/// Routes an incoming [`ClientMessage`] to the appropriate handler.
/// Returns a [`ServerMessage`] response, or `None` if the operation fails.
async fn handle_client_message(
    msg: ClientMessage,
    state: &SharedServerState,
) -> Option<ServerMessage> {
    match msg {
        ClientMessage::CreateRoom { player_name } => create_room(player_name, state).await,
        ClientMessage::JoinRoom {
            room_code,
            player_name,
        } => join_room(room_code, player_name, state).await,
    }
}

/// Creates a new room with the given player as host.
/// Returns a [`ServerMessage::RoomCreated`] with the room code and initial player list.
async fn create_room(player_name: String, state: &SharedServerState) -> Option<ServerMessage> {
    let player_id = uuid::Uuid::new_v4();
    let room_code = generate_room_code();
    let player = Player {
        id: player_id,
        name: player_name,
    };
    let players = vec![player];

    let room = Room::shared(player_id, players.clone());
    // Hold lock on room map only long enough to insert a room, then release it
    state.rooms.write().await.insert(room_code.clone(), room);

    Some(ServerMessage::RoomCreated {
        room_code,
        my_player_id: player_id,
        players,
    })
}

/// Adds a new player to an existing room.
/// Returns `None` if the room code is not found.
/// Returns a [`ServerMessage::RoomJoined`] with the current player list on success.
async fn join_room(
    room_code: String,
    player_name: String,
    state: &SharedServerState,
) -> Option<ServerMessage> {
    let player_id = uuid::Uuid::new_v4();
    let player = Player {
        id: player_id,
        name: player_name,
    };

    // Read-lock the room map, get and clone the room's Arc, then release the lock on the room map
    let room_arc = state.rooms.read().await.get(&room_code)?.clone();
    // Write-lock the room so we can mutate it
    let mut room = room_arc.write().await;
    room.players.push(player);

    Some(ServerMessage::RoomJoined {
        room_code,
        host_id: room.host_id,
        my_player_id: player_id,
        players: room.players.clone(),
    })
}

/// Generates a random 4-character uppercase room code (e.g. `"XKQZ"`).
fn generate_room_code() -> String {
    use rand::RngExt;
    let mut rng = rand::rng();
    (0..4)
        .map(|_| rng.random_range(b'A'..=b'Z') as char)
        .collect()
}
