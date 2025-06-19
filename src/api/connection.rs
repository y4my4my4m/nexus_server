// server/src/api/connection.rs

use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Mutex};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use uuid::Uuid;
use futures::{SinkExt, StreamExt};
use tracing::{error, info};
use common::{ClientMessage, ServerMessage, User};

use crate::db;
use crate::db::users::*;
use crate::db::forums::*;
use crate::db::servers::*;
use crate::db::channels::*;
use crate::db::messages::*;
use crate::db::notifications::*;
use crate::util::*;
use crate::auth::*;

// Peer and PeerMap types for session management
pub struct Peer {
    pub user_id: Option<Uuid>,
    pub tx: mpsc::UnboundedSender<ServerMessage>,
}

pub type PeerMap = Arc<Mutex<HashMap<Uuid, Peer>>>;

// Main connection handler (moved from main.rs)
pub async fn handle_connection(
    stream: TcpStream,
    peer_map: PeerMap,
) {
    let peer_id = Uuid::new_v4();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let user_id: Option<Uuid>;

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
    // Spawn a task to handle incoming messages
    tokio::spawn(async move {
        while let Some(Ok(msg)) = stream.next().await {
            match serde_json::from_slice::<ClientMessage>(&msg) {
                Ok(message) => {
                    match message {
                        ClientMessage::Register { username, password } => {
                            let result = db::users::db_register_user(&username, &password, "Reset", "User").await;
                            // No RegistrationSuccess or Error variant, so just log or send a generic message
                            // TODO: Implement proper response
                        }
                        ClientMessage::Login { username, password } => {
                            let result = db::users::db_login_user(&username, &password).await;
                            // No LoginSuccess or Error variant, so just log or send a generic message
                            // TODO: Implement proper response
                        }
                        ClientMessage::Logout => {
                            let mut peers = peer_map_task.lock().await;
                            if let Some(peer) = peers.get_mut(&peer_id) {
                                peer.user_id = None;
                            }
                            // No LogoutSuccess variant, so just log
                        }
                        ClientMessage::CreatePost { thread_id, content } => {
                            let peers = peer_map_task.lock().await;
                            if let Some(peer) = peers.get(&peer_id) {
                                if let Some(user_id) = peer.user_id {
                                    let _ = db::forums::db_create_post(thread_id, user_id, &content).await;
                                    // No PostCreated variant, so just log
                                }
                            }
                        }
                        ClientMessage::CreateThread { forum_id, title, content } => {
                            let peers = peer_map_task.lock().await;
                            if let Some(peer) = peers.get(&peer_id) {
                                if let Some(user_id) = peer.user_id {
                                    let _ = db::forums::db_create_thread(forum_id, &title, user_id, &content).await;
                                    // No ThreadCreated variant, so just log
                                }
                            }
                        }
                        _ => todo!(),
                    }
                }
                Err(e) => {
                    error!("Error parsing message: {:?}", e);
                }
            }
        }
    });

    // Main loop to send messages to the client
    while let Some(msg) = rx.recv().await {
        if let Err(e) = sink.send(serde_json::to_vec(&msg).unwrap().into()).await {
            error!("Error sending message: {:?}", e);
            break;
        }
    }

    // Cleanup on disconnect
    {
        let mut peers = peer_map.lock().await;
        peers.remove(&peer_id);
    }
}
