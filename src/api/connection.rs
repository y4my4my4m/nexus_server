use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::TcpStream;
use crate::errors::Result;
use tokio::sync::{mpsc, Mutex};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use uuid::Uuid;
use futures::{SinkExt, StreamExt};
use tracing::error;
use common::{ClientMessage, ServerMessage};

use crate::api::routes::MessageRouter;
use crate::db;
use crate::services::BroadcastService;

/// Represents a connected peer/client
pub struct Peer {
    pub user_id: Option<Uuid>,
    pub tx: mpsc::UnboundedSender<ServerMessage>,
}

/// Thread-safe map of all connected peers
pub type PeerMap = Arc<Mutex<HashMap<Uuid, Peer>>>;

/// Main connection handler - processes client connections and messages
pub async fn handle_connection(
    stream: TcpStream,
    peer_map: PeerMap,
) -> Result<()> {
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
                Some(Ok(msg)) = stream.next() => {
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
                Some(msg) = rx.recv() => {
                    tracing::info!("Sending ServerMessage: {:?}", msg);
                    if let Err(e) = sink.send(bincode::serialize(&msg).unwrap().into()).await {
                        error!("Error sending message: {:?}", e);
                        break;
                    }
                }
                else => { break; }
            }
        }
        
        // Cleanup on disconnect
        let mut peers = peer_map_task.lock().await;
        let user_id_opt = peers.get(&peer_id).and_then(|p| p.user_id);
        peers.remove(&peer_id);
        drop(peers);
        
        if let Some(user_id) = user_id_opt {
            if let Ok(profile) = db::users::db_get_user_by_id(user_id).await {
                let user = common::User {
                    id: profile.id,
                    username: profile.username,
                    color: profile.color,
                    role: profile.role,
                    profile_pic: profile.profile_pic,
                    cover_banner: profile.cover_banner,
                    status: common::UserStatus::Connected,
                };
                BroadcastService::broadcast_user_status_change(&peer_map_task, &user, false).await;
            }
        }
    });
    
    Ok(())
}
