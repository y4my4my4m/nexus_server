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
use std::sync::Arc;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use tokio_rustls::rustls::{ServerConfig as RustlsServerConfig};
use rustls_pemfile::{certs, pkcs8_private_keys};
use std::fs::File;
use std::io::BufReader;

fn load_certs(path: &str) -> Vec<CertificateDer<'static>> {
    let certfile = File::open(path).expect("Cannot open certificate file");
    let mut reader = BufReader::new(certfile);
    certs(&mut reader)
        .filter_map(|res| res.ok())
        .collect()
}

fn load_private_key(path: &str) -> PrivatePkcs8KeyDer<'static> {
    let keyfile = File::open(path).expect("Cannot open private key file");
    let mut reader = BufReader::new(keyfile);
    let keys: Vec<_> = pkcs8_private_keys(&mut reader)
        .filter_map(|res| res.ok())
        .collect();
    keys.into_iter().next().expect("No private key found in file")
}

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
    info!("🚀 Nexus Server listening on: {} (TLS enabled)", addr);

    // Load TLS config
    let certs = load_certs("cert.pem");
    let key = load_private_key("key.pem");
    let tls_config = RustlsServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, tokio_rustls::rustls::pki_types::PrivateKeyDer::Pkcs8(key))?;
    let tls_acceptor = TlsAcceptor::from(Arc::new(tls_config));

    // Initialize peer map for connection management
    let peer_map = PeerMap::new(Mutex::new(HashMap::new()));

    // Accept connections
    loop {
        let (stream, _) = listener.accept().await?;
        let peer_map = peer_map.clone();
        let tls_acceptor = tls_acceptor.clone();
        tokio::spawn(async move {
            match tls_acceptor.accept(stream).await {
                Ok(tls_stream) => {
                    if let Err(e) = handle_connection(tls_stream, peer_map).await {
                        error!("Connection error: {}", e);
                    }
                }
                Err(e) => {
                    error!("TLS handshake failed: {}", e);
                }
            }
        });
    }
}