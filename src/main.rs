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
///
/// Phase 1: waits for the first valid `CreateRoom` or `JoinRoom` message,
/// sends the direct response, and captures a broadcast receiver for the room.
///
/// Phase 2: runs a `select!` loop racing incoming client messages against
/// room broadcasts, forwarding both to the WebSocket sender.
async fn handle_socket(socket: WebSocket, state: SharedServerState) {
    let (mut sender, mut receiver) = socket.split();

    // Phase 1: Wait for the client to send a valid CreateRoom or JoinRoom message.
    // All other messages are ignored. The loop keeps running until the client
    // either joins a room or closes the connection.
    let mut broadcast_rx = loop {
        let client_event = next_client_event(&mut receiver).await;
        match process_client_event(client_event, &state, &mut sender).await {
            Ok(Some(rx)) => break rx,  // Room joined, broadcast receiver is ready.
            Ok(None) => continue,                     // Unhandled message, keep waiting.
            Err(_) => return,                         // Connection closed or error, exit
        }
    };

    // Phase 2: Wait for either a client event or a room broadcast event, whichever arrives first.
    // Once an event arrives, it is handled and the loop waits again.
    loop {
        tokio::select! {
            client_event = next_client_event(&mut receiver) => {
                match process_client_event(client_event, &state, &mut sender).await {
                    Ok(_) => {}       // Message handled or skipped, keep going.
                    Err(_) => return, // Connection closed or error, exit.
                }
            }
            broadcast_event = broadcast_rx.recv() => {
                match process_broadcast_event(broadcast_event, &mut sender).await {
                    Ok(_) => {}       // Broadcast forwarded or lagged, keep going.
                    Err(_) => return, // Channel closed or error, exit.
                }
            }
        }
    }
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

/// Processes a [`ClientEvent`], optionally returning a broadcast receiver if a room was joined or created.
///
/// - [`ClientEvent::Message`] is dispatched to [`handle_client_message`]. If it results in a room
///   being joined or created, the response is sent and the room's broadcast receiver is returned.
/// - [`ClientEvent::Skip`] returns `Ok(None)` — the message was unrecognized or malformed.
/// - [`ClientEvent::Close`] returns `Err` to signal the connection should be closed.
///
/// Returns `Ok(None)` if no room was joined, `Ok(Some(rx))` if a room was joined or created,
/// and `Err` if the connection should be closed.
async fn process_client_event(
    event: ClientEvent,
    state: &SharedServerState,
    sender: &mut SplitSink<WebSocket, Message>,
) -> anyhow::Result<Option<broadcast::Receiver<String>>> {
    match event {
        ClientEvent::Skip => Ok(None),
        ClientEvent::Close => anyhow::bail!("connection closed"),
        ClientEvent::Message(client_msg) => {
            if let Some((response, rx)) = handle_client_message(client_msg, state).await {
                send_response(sender, &response).await?;
                Ok(Some(rx))
            } else {
                Ok(None)
            }
        }
    }
}

/// Routes an incoming [`ClientMessage`] to the appropriate handler.
/// Returns a [`ServerMessage`] and a broadcast receiver for the player's room,
/// or `None` if the operation fails (e.g., room not found).
async fn handle_client_message(
    msg: ClientMessage,
    state: &SharedServerState,
) -> Option<(ServerMessage, broadcast::Receiver<String>)> {
    match msg {
        ClientMessage::CreateRoom { player_name } => create_room(player_name, state).await,
        ClientMessage::JoinRoom {
            room_code,
            player_name,
        } => join_room(room_code, player_name, state).await,
    }
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
async fn send_response(sender: &mut SplitSink<WebSocket, Message>, response: &ServerMessage) -> anyhow::Result<()> {
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
) -> Option<(ServerMessage, broadcast::Receiver<String>)> {
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
            room_code,
            my_player_id: player_id,
            players,
        },
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
) -> Option<(ServerMessage, broadcast::Receiver<String>)> {
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
            room_code,
            host_id: room.host_id,
            my_player_id: player_id,
            players: all_players,
        },
        rx,
    ))
}

/// Generates a random 4-character uppercase room code (e.g. `"XKQZ"`).
fn generate_room_code() -> String {
    use rand::RngExt;
    let mut rng = rand::rng();
    (0..4)
        .map(|_| rng.random_range(b'A'..=b'Z') as char)
        .collect()
}
