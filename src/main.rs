mod api;
mod db;
mod util;
mod auth;
mod services;
mod errors;

use api::connection::{handle_connection, PeerMap};
use db::db_config;
use db::migrations::init_db;
use db::servers::ensure_default_server_exists;
use std::collections::HashMap;
use std::env;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tracing::{error, info};
use common::config::ServerConfig;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();
    
    // Load server configuration
    let config_path = env::args().nth(2).unwrap_or_else(|| "server_config.toml".to_string());
    let config = ServerConfig::load_or_default(&config_path);
    info!("Loaded configuration from {}", config_path);
    
    // Initialize global database path from configuration
    db_config::init_db_path(config.database.path.clone());
    info!("Database path set to: {}", config.database.path);
    
    // Get server address
    let addr = env::args()
        .nth(1)
        .unwrap_or_else(|| format!("{}:{}", config.network.bind_address, config.network.port));
    
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
    info!("🚀 Nexus Server listening on: {}", addr);
    
    // Initialize peer map for connection management
    let peer_map = PeerMap::new(Mutex::new(HashMap::new()));

    // Accept connections
    loop {
        let (stream, _) = listener.accept().await?;
        let peer_map = peer_map.clone();
        
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, peer_map).await {
                error!("Connection error: {}", e);
            }
        });
    }
}