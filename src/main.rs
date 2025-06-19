// server/src/main.rs

mod api;
mod db;
mod util;
mod auth;

use api::connection::{handle_connection, PeerMap};
// use db::migrations::init_db;
// use db::servers::ensure_default_server_exists;
use std::collections::HashMap;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    let addr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:8080".to_string());
    let listener = TcpListener::bind(&addr).await?;
    info!("Server listening on: {}", addr);
    let peer_map = PeerMap::new(Mutex::new(HashMap::new()));
    // Initialize the database
    // let _conn = init_db()?;
    // Ensure default server and channels exist
    // ensure_default_server_exists().await.map_err(|e| format!("Failed to create default server: {}", e))?;
    loop {
        let (stream, _) = listener.accept().await?;
        tokio::spawn(handle_connection(stream, peer_map.clone()));
    }
}