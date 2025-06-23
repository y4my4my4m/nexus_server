use std::collections::HashMap;
use std::sync::Arc;
use std::error::Error;
use tokio::net::TcpStream;
use crate::errors::Result;
use tokio::sync::{mpsc, Mutex};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use uuid::Uuid;
use futures::{SinkExt, StreamExt};
use tracing::{error, info};
use common::{ClientMessage, ServerMessage};

use crate::api::routes::MessageRouter;
use crate::db;
use crate::services::BroadcastService;
use tokio_rustls::server::TlsStream;
use tokio::io::{AsyncRead, AsyncWrite};

/// Represents a connected peer/client
pub struct Peer {
    pub user_id: Option<Uuid>,
    pub tx: mpsc::UnboundedSender<ServerMessage>,
}

/// Thread-safe map of all connected peers
pub type PeerMap = Arc<Mutex<HashMap<Uuid, Peer>>>;

/// Handle user disconnect and broadcast status change
async fn handle_user_disconnect(peer_map: &PeerMap, peer_id: Uuid, reason: &str) {
    info!("Handling user disconnect for peer {}: {}", peer_id, reason);
    
    // Get user info before cleanup
    let user_id_opt = {
        let peers = peer_map.lock().await;
        peers.get(&peer_id).and_then(|p| p.user_id)
    };
    
    // Broadcast user disconnect if they were authenticated
    if let Some(user_id) = user_id_opt {
        if let Ok(profile) = db::users::db_get_user_by_id(user_id).await {
            let user = common::User {
                id: profile.id,
                username: profile.username.clone(),
                color: profile.color,
                role: profile.role,
                profile_pic: profile.profile_pic,
                cover_banner: profile.cover_banner,
                status: common::UserStatus::Offline,
            };
            
            info!("Broadcasting disconnect for user: {} ({})", user.username, reason);
            BroadcastService::broadcast_user_status_change(peer_map, &user, false).await;
        }
    }
}

/// Main connection handler - processes client connections and messages
pub async fn handle_connection<S>(
    stream: S,
    peer_map: PeerMap,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let peer_id = Uuid::new_v4();
    let (tx, mut rx) = mpsc::unbounded_channel();

    {
        let mut peers = peer_map.lock().await;
        peers.insert(
            peer_id,
            Peer {
                user_id: None,
                tx: tx.clone(),
            },
        );
    }

    let framed = Framed::new(stream, LengthDelimitedCodec::new());
    let (mut sink, mut stream) = framed.split();

    let peer_map_task = peer_map.clone();
    tokio::spawn(async move {
        let mut current_user: Option<common::User> = None;
        let router = MessageRouter::new(peer_map_task.clone());
        
        loop {
            tokio::select! {
                stream_result = stream.next() => {
                    match stream_result {
                        Some(Ok(msg)) => {
                            match bincode::deserialize::<ClientMessage>(&msg) {
                                Ok(message) => {
                                    tracing::info!("Parsed ClientMessage: {:?}", message);
                                    
                                    // Use the router to handle the message
                                    if let Err(e) = router.handle_message(
                                        message,
                                        &mut current_user,
                                        peer_id,
                                        &tx,
                                    ).await {
                                        error!("Error handling message: {:?}", e);
                                    }
                                }
                                Err(e) => {
                                    error!("Error parsing message: {:?}", e);
                                }
                            }
                        }
                        Some(Err(e)) => {
                            // Handle stream errors (connection issues, broken pipe from read side)
                            error!("Stream error: {:?}", e);
                            handle_user_disconnect(&peer_map_task, peer_id, "stream error").await;
                            break;
                        }
                        None => {
                            // Stream ended
                            handle_user_disconnect(&peer_map_task, peer_id, "stream ended").await;
                            break;
                        }
                    }
                }
                Some(msg) = rx.recv() => {
                    // tracing::debug!("Sending ServerMessage: {:?}", msg);
                    if let Err(e) = sink.send(bincode::serialize(&msg).unwrap().into()).await {
                        error!("Error sending message: {:?}", e);
                        
                        // Check if it's a broken pipe error for immediate handling
                        if let Some(io_error) = e.source().and_then(|e| e.downcast_ref::<std::io::Error>()) {
                            if io_error.kind() == std::io::ErrorKind::BrokenPipe {
                                handle_user_disconnect(&peer_map_task, peer_id, "broken pipe").await;
                            }
                        }
                        break;
                    }
                }
                else => { break; }
            }
        }
        
        // Final cleanup - remove from peer map and handle any remaining disconnect
        let was_authenticated = {
            let mut peers = peer_map_task.lock().await;
            let was_auth = peers.get(&peer_id).and_then(|p| p.user_id).is_some();
            peers.remove(&peer_id);
            was_auth
        };
        
        // Only do final disconnect handling if we haven't already handled it above
        if was_authenticated {
            handle_user_disconnect(&peer_map_task, peer_id, "connection cleanup").await;
        }
    });
    
    Ok(())
}
