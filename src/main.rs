use axum::{routing::get, Router};
use std::net::SocketAddr;

#[tokio::main]
async fn main() {
    // Initialize the Axum router
    let app = Router::new()
        .route("/", get(root_handler));

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