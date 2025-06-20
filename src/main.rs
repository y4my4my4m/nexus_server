// server/src/main.rs

mod api;
mod db;
mod util;
mod auth;
mod services;
mod errors;

use api::connection::{handle_connection, PeerMap};
use db::migrations::init_db;
use db::servers::ensure_default_server_exists;
use std::collections::HashMap;
use std::env;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();
    
    // Get server address
    let addr = env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:8080".to_string());
    
    // Initialize the database
    match init_db().await {
        Ok(_) => info!("Database initialized successfully"),
        Err(e) => {
            error!("Failed to initialize database: {}", e);
            return Err(e.into());
        }
    }
    
    // Ensure default server and channels exist
    if let Err(e) = ensure_default_server_exists().await {
        error!("Failed to create default server: {}", e);
        return Err(e.into());
    }
    
    // Start TCP listener
    let listener = TcpListener::bind(&addr).await?;
    info!("🚀 Cyberpunk BBS Server listening on: {}", addr);
    
    // Initialize peer map for connection management
    let peer_map = PeerMap::new(Mutex::new(HashMap::new()));
    
    // Main server loop
    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                info!("New connection from: {}", addr);
                let peer_map_clone = peer_map.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(stream, peer_map_clone).await {
                        error!("Connection error: {}", e);
                    }
                });
            }
            Err(e) => {
                error!("Failed to accept connection: {}", e);
            }
        }
    }
}