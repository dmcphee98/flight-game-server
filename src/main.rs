mod messages;
mod state;

use crate::messages::ClientEvent;
use axum::extract::ws::{Message, WebSocket};
use axum::{
    Router,
    extract::{State, WebSocketUpgrade},
    routing::get,
};
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use messages::{ClientMessage, ServerMessage};
use state::{Player, Room, ServerState, SharedServerState};
use std::{net::SocketAddr, sync::Arc};
use tokio::sync::broadcast;

#[tokio::main]
async fn main() {
    let state: SharedServerState = Arc::new(ServerState::default());

    let app = Router::new()
        .route("/", get(root_handler))
        .route("/ws", get(websocket_handler))
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
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
///
/// Phase 1: waits for the first valid `CreateRoom` or `JoinRoom` message,
/// sends the direct response, and captures a broadcast receiver for the room.
///
/// Phase 2: runs a `select!` loop racing incoming client messages against
/// room broadcasts, forwarding both to the WebSocket sender.
async fn handle_socket(socket: WebSocket, state: SharedServerState) {
    let (mut sender, mut receiver) = socket.split();

    // Phase 1: Wait for a CreateRoom or JoinRoom event.
    // Continues to loop until either one is received, ignoring all other ClientMessage types.
    let (player_id, room_code, mut broadcast_rx) = loop {
        let client_event = next_client_event(&mut receiver).await;
        let client_message = match classify_client_event(client_event) {
            Ok(Some(msg)) => msg,
            Ok(None) => continue,
            Err(_) => return,
        };
        let result = match client_message {
            ClientMessage::CreateRoom { player_name } => create_room(player_name, &state).await,
            ClientMessage::JoinRoom {
                room_code,
                player_name,
            } => join_room(room_code, player_name, &state).await,
            _ => None,
        };
        if let Some((response, player_id, room_code, broadcast_rx)) = result {
            let _ = send_response(&mut sender, &response).await;
            break (player_id, room_code, broadcast_rx);
        }
    };

    // Phase 2: Wait for either a client event or a room broadcast event, whichever arrives first.
    // Once an event arrives, it is handled and the loop waits again.
    loop {
        tokio::select! {
            client_event = next_client_event(&mut receiver) => {
                match process_client_event(client_event, player_id, &room_code, &state).await {
                    Ok(_) => continue, // Client event processed, continue looping.
                    Err(_) => break,   // Channel closed or error, exit.
                }
            }
            broadcast_event = broadcast_rx.recv() => {
                match process_broadcast_event(broadcast_event, &mut sender).await {
                    Ok(_) => continue, // Broadcast processed or lagged, continue looping.
                    Err(_) => break,   // Channel closed or error, exit.
                }
            }
        }
    }

    disconnect_player(player_id, &room_code, &state).await;
}

/// Reads the next message from the WebSocket stream and classifies it as a [`ClientEvent`].
///
/// - Valid text frames are deserialized into [`ClientEvent::Message`].
/// - Close frames and a closed stream return [`ClientEvent::Close`].
/// - Unrecognized frame types or malformed frames return [`ClientEvent::Skip`].
async fn next_client_event(receiver: &mut SplitStream<WebSocket>) -> ClientEvent {
    match receiver.next().await {
        // Parse 'text' websocket message into ClientMessage.
        Some(Ok(Message::Text(text))) => match serde_json::from_str::<ClientMessage>(&text) {
            Ok(msg) => ClientEvent::Message(msg),
            Err(_) => ClientEvent::Skip,
        },
        // Parse 'close' websocket message.
        Some(Ok(Message::Close(_))) | None => ClientEvent::Close,
        _ => ClientEvent::Skip,
    }
}

/// Classifies a raw [`ClientEvent`] into a structured result for the game loop.
///
/// Returns:
/// - `Ok(Some(msg))` — a valid message ready to be handled
/// - `Ok(None)` — a non-fatal event to skip (e.g. a malformed message)
/// - `Err` — the connection was closed; the caller should exit the loop
fn classify_client_event(event: ClientEvent) -> anyhow::Result<Option<ClientMessage>> {
    match event {
        ClientEvent::Skip => Ok(None),
        ClientEvent::Close => anyhow::bail!("connection closed"),
        ClientEvent::Message(msg) => Ok(Some(msg)),
    }
}

/// Processes a single [`ClientEvent`] from a player in an active game session.
///
/// Classifies the event via [`classify_client_event`], then dispatches to the
/// appropriate handler based on the message type. Messages that are not valid
/// in the game phase (e.g. `CreateRoom`) are silently ignored.
///
/// Returns `Err` if the connection was closed, signalling the caller to exit the loop.
async fn process_client_event(
    event: ClientEvent,
    player_id: uuid::Uuid,
    room_code: &str,
    state: &SharedServerState,
) -> anyhow::Result<()> {
    let Some(msg) = classify_client_event(event)? else {
        return Ok(());
    };
    match msg {
        ClientMessage::StartGame => start_game(player_id, room_code, state).await,
        _ => {}
    }
    Ok(())
}

/// Handles an event received from a room's broadcast channel.
///
/// - `Ok(json)` — forwards the message to the client.
/// - `Err(RecvError::Lagged(n))` — the receiver fell behind and missed `n` messages;
///   logged but not fatal, recv() will resume from the oldest surviving message.
/// - `Err(RecvError::Closed)` — the room no longer exists; returns `Err` to signal
///   the connection should be closed.
async fn process_broadcast_event(
    broadcast_event: Result<String, broadcast::error::RecvError>,
    sender: &mut SplitSink<WebSocket, Message>,
) -> anyhow::Result<()> {
    match broadcast_event {
        Ok(json) => {
            sender.send(Message::Text(json.into())).await?;
            Ok(())
        }
        Err(broadcast::error::RecvError::Lagged(n)) => {
            eprintln!("broadcast receiver lagged by {} messages", n);
            Ok(())
        }
        Err(broadcast::error::RecvError::Closed) => anyhow::bail!("broadcast channel closed"),
    }
}

/// Serializes a [`ServerMessage`] to JSON and sends it to the client as a WebSocket text frame.
///
/// Returns `Err` if serialization fails or the WebSocket send fails.
async fn send_response(
    sender: &mut SplitSink<WebSocket, Message>,
    response: &ServerMessage,
) -> anyhow::Result<()> {
    let json = serde_json::to_string(&response)?;
    sender.send(Message::Text(json.into())).await?;
    Ok(())
}

/// Creates a new room with the given player as host.
/// Subscribes the host to the room's broadcast channel so they receive
/// future `PlayerJoined` events.
async fn create_room(
    player_name: String,
    state: &SharedServerState,
) -> Option<(
    ServerMessage,
    uuid::Uuid,
    String,
    broadcast::Receiver<String>,
)> {
    let player_id = uuid::Uuid::new_v4();
    let room_code = generate_room_code();
    let player = Player {
        id: player_id,
        name: player_name,
    };
    let players = vec![player];

    let room = Room::shared(player_id, players.clone());
    let rx = room.read().await.tx.subscribe();
    // Hold lock on room map only long enough to insert, then release it.
    state.rooms.write().await.insert(room_code.clone(), room);

    Some((
        ServerMessage::RoomCreated {
            room_code: room_code.clone(),
            my_player_id: player_id,
            players,
        },
        player_id,
        room_code,
        rx,
    ))
}

/// Adds a new player to an existing room and broadcasts a `PlayerJoined` event
/// to all currently subscribed players.
///
/// The joining player subscribes **after** the broadcast, so they do not receive
/// their own join event. They already receive the full state in `RoomJoined`.
///
/// Returns `None` if the room code is not found.
async fn join_room(
    room_code: String,
    player_name: String,
    state: &SharedServerState,
) -> Option<(
    ServerMessage,
    uuid::Uuid,
    String,
    broadcast::Receiver<String>,
)> {
    let player_id = uuid::Uuid::new_v4();
    let player = Player {
        id: player_id,
        name: player_name,
    };

    let room_arc = state.rooms.read().await.get(&room_code)?.clone();
    let mut room = room_arc.write().await;
    room.players.push(player.clone());
    let all_players = room.players.clone();

    // Broadcast to existing players before subscribing the new player,
    // so the new player does not receive their own join event.
    let broadcast_msg = ServerMessage::PlayerJoined {
        player: player.clone(),
        players: all_players.clone(),
    };
    let _ = room.tx.send(serde_json::to_string(&broadcast_msg).unwrap());

    let rx = room.tx.subscribe();

    Some((
        ServerMessage::RoomJoined {
            room_code: room_code.clone(),
            host_id: room.host_id,
            my_player_id: player_id,
            players: all_players,
        },
        player_id,
        room_code,
        rx,
    ))
}

/// Starts the game for a room if the requesting player is the host and the game
/// has not already started. Broadcasts `GameStarted` to all players with a
/// timestamp 5 seconds in the future.
async fn start_game(player_id: uuid::Uuid, room_code: &str, state: &SharedServerState) {
    let room_arc = match state.rooms.read().await.get(room_code).cloned() {
        Some(r) => r,
        None => return,
    };

    let mut room = room_arc.write().await;

    if room.host_id != player_id || room.started {
        return;
    }

    room.started = true;

    let starts_at_unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        + 5_000;

    let msg = ServerMessage::GameStarted { starts_at_unix_ms };
    let _ = room.tx.send(serde_json::to_string(&msg).unwrap());
}

/// Removes the player from their room and broadcasts a `PlayerDisconnected` event
/// to all remaining players. If the room is now empty, removes it from the map.
async fn disconnect_player(player_id: uuid::Uuid, room_code: &str, state: &SharedServerState) {
    let room_arc = match state.rooms.read().await.get(room_code).cloned() {
        Some(r) => r,
        None => return,
    };

    let remaining = {
        let mut room = room_arc.write().await;
        room.players.retain(|p| p.id != player_id);
        let remaining = room.players.clone();
        let msg = ServerMessage::PlayerDisconnected {
            player_id,
            players: remaining.clone(),
        };
        let _ = room.tx.send(serde_json::to_string(&msg).unwrap());
        remaining
    };

    if remaining.is_empty() {
        state.rooms.write().await.remove(room_code);
    }
}

/// Generates a random 4-character uppercase room code (e.g. `"XKQZ"`).
fn generate_room_code() -> String {
    use rand::RngExt;
    let mut rng = rand::rng();
    (0..4)
        .map(|_| rng.random_range(b'A'..=b'Z') as char)
        .collect()
}
