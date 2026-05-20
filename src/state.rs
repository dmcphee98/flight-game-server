use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct Player {
    pub id: Uuid,
    pub name: String,
}

pub struct Room {
    pub host_id: Uuid,
    pub players: Vec<Player>,
}

impl Room {
    /// Creates a new [`SharedRoom`] with the given host and initial players.
    pub fn shared(host_id: Uuid, players: Vec<Player>) -> SharedRoom {
        Arc::new(RwLock::new(Self { host_id, players }))
    }
}

/// A shared, thread-safe handle to a single room.
///
/// `RwLock` ensures safe concurrent access to the room's data. Multiple tasks
/// can read the room simultaneously, but a task that needs to write to it (e.g.,
/// a player joining) gets exclusive access until it is done.
///
/// `Arc` allows a room to be held independently of the `HashMap` that owns it.
/// Once the `Arc` has been cloned, the `HashMap` lock can be released while
/// the room continues to be accessed without blocking other tasks.
pub type SharedRoom = Arc<RwLock<Room>>;

/// A shared, thread-safe map of room codes to rooms.
///
/// `RwLock` allows multiple tasks to look up rooms simultaneously, while
/// ensuring exclusive access when a room is being added or removed.
///
/// For operations on a specific room's data, see [`SharedRoom`].
pub type SharedRoomMap = RwLock<HashMap<String, SharedRoom>>;

/// The global server state.
///
/// [`SharedRoomMap`] requires a write lock only when a room is created or
/// removed — lookups acquire a read lock, allowing multiple tasks to find
/// rooms simultaneously.
///
/// Each [`SharedRoom`] is locked independently, so tasks operating on
/// different rooms never block each other.
pub struct ServerState {
    pub rooms: SharedRoomMap,
}

impl Default for ServerState {
    fn default() -> Self {
        Self {
            rooms: RwLock::new(HashMap::new()),
        }
    }
}

/// A shared handle to the global server state, passed to every task and handler.
///
/// `Arc` allows multiple tasks to hold a reference to the same `ServerState`
/// without copying it. Locking is handled by the fields within `ServerState`
/// itself. See [`SharedRoomMap`] and [`SharedRoom`].
pub type SharedServerState = Arc<ServerState>;
