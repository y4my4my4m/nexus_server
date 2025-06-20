mod api;
mod db;
mod util;
mod auth;
mod services;
mod errors;

use api::connection::{handle_connection, PeerMap};
use db::migrations::init_db;
use db::servers::ensure_default_server_exists;
use services::{RateLimitService, ContentFilterService, AuditService, AuditAction};
use common::config::ServerConfig;
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::{Mutex, RwLock};
use tracing::{error, info, warn};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();
    
    // Load server configuration
    let config_path = env::args().nth(2).unwrap_or_else(|| "server_config.toml".to_string());
    let config = ServerConfig::load_or_default(&config_path);
    
    // Validate configuration
    if let Err(e) = config.validate() {
        error!("Invalid server configuration: {}", e);
        return Err(e.into());
    }
    
    info!("Loaded server configuration from {}", config_path);
    info!("Server settings:");
    info!("  - Bind address: {}:{}", config.network.bind_address, config.network.port);
    info!("  - Max connections: {}", config.network.max_connections);
    info!("  - Rate limits: {} msg/min, {} req/sec", 
          config.rate_limits.messages_per_minute, 
          config.rate_limits.requests_per_second);
    info!("  - Auto-moderation: {}", config.moderation.auto_moderation_enabled);
    info!("  - Audit logging: {}", config.security.audit_logging_enabled);
    
    // Initialize services
    let rate_limit_service = Arc::new(RateLimitService::new(config.rate_limits.clone()));
    let content_filter_service = Arc::new(RwLock::new(
        ContentFilterService::new(config.moderation.clone())
            .map_err(|e| format!("Failed to initialize content filter: {}", e))?
    ));
    
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
    
    // Log server startup
    if config.security.audit_logging_enabled {
        AuditService::log_action(
            AuditAction::ConfigurationChanged,
            None,
            None,
            None,
            None,
            std::collections::HashMap::new(),
            Some("Server started".to_string()),
        ).await.unwrap_or_else(|e| warn!("Failed to log server startup: {}", e));
    }
    
    // Start TCP listener
    let listener = TcpListener::bind(&addr).await?;
    info!("🚀 Nexus Server listening on: {}", addr);
    
    // Initialize peer map for connection management
    let peer_map = PeerMap::new(Mutex::new(HashMap::new()));
    
    // Start rate limit cleanup task
    let rate_limit_cleanup = rate_limit_service.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(300)); // 5 minutes
        loop {
            interval.tick().await;
            rate_limit_cleanup.cleanup_old_entries().await;
        }
    });
    
    // Start configuration backup task
    let config_backup = config.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(
            tokio::time::Duration::from_secs(config_backup.database.backup_interval_hours * 3600)
        );
        loop {
            interval.tick().await;
            // Implement database backup logic here
            info!("Performing scheduled database backup...");
        }
    });
    
    // Accept connections
    while let Ok((socket, addr)) = listener.accept().await {
        info!("🔗 New connection from: {}", addr);
        
        let peer_map = peer_map.clone();
        let rate_limit_service = rate_limit_service.clone();
        let content_filter_service = content_filter_service.clone();
        let server_config = config.clone();
        
        tokio::spawn(async move {
            if let Err(e) = handle_connection(
                socket, 
                addr, 
                peer_map, 
                rate_limit_service, 
                content_filter_service,
                server_config
            ).await {
                error!("Connection error: {}", e);
            }
        });
    }

    Ok(())
}