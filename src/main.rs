use axum::{routing::get, Router};
use std::net::SocketAddr;
use axum::extract::WebSocketUpgrade;
use axum::extract::ws::{Message, WebSocket};

#[tokio::main]
async fn main() {
    // Initialize the Axum router
    let app = Router::new()
        .route("/", get(root_handler))
        .route("/ws", get(websocket_handler));

    // Specify the address to bind to
    let addr = SocketAddr::from(([127, 0, 0, 1], 8080));

    // Start the Axum server
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    println!("Server running at {}", addr);
    axum::serve(listener, app).await.unwrap();
}

async fn root_handler() -> &'static str {
    "Hello, world!"
}

async fn websocket_handler(ws: WebSocketUpgrade) -> impl axum::response::IntoResponse {
    ws.on_upgrade(handle_socket)
}

async fn handle_socket(mut socket: WebSocket) {
    // Send a greeting message to the client
    if let Err(e) = socket
        .send(Message::Text("Hello from the server!".into()))
        .await
    {
        eprintln!("Error sending message: {}", e);
        return;
    }

    // Loop to keep the connection alive
    while let Some(Ok(msg)) = socket.recv().await {
        match msg {
            Message::Text(msg) => {
                println!("Received message: {}", msg);
                if let Err(e) = socket
                    .send(Message::Text(format!("Echo: {}", msg).into()))
                    .await
                {
                    eprintln!("Error sending message: {}", e);
                }
            }
            Message::Close(_) => {
                println!("Closing WebSocket connection.");
                break;
            }
            _ => {}
        }
    }
}