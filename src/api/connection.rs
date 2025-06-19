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

    // Spawn a task to handle incoming messages
    tokio::spawn(async move {
        while let Some(Ok(msg)) = stream.next().await {
            match serde_json::from_slice::<ClientMessage>(&msg) {
                Ok(message) => {
                    match message {
                        ClientMessage::Ping => {
                            // Handle ping message
                            let _ = sink.send(serde_json::to_vec(&ServerMessage::Pong).unwrap().into()).await;
                        }
                        ClientMessage::Register { username, password } => {
                            // Handle user registration
                            let result = db::register_user(&username, &password).await;
                            let response = match result {
                                Ok(user) => ServerMessage::RegistrationSuccess { user_id: user.id },
                                Err(e) => {
                                    error!("Registration error: {:?}", e);
                                    ServerMessage::Error { message: "Registration failed".into() }
                                }
                            };
                            let _ = sink.send(serde_json::to_vec(&response).unwrap().into()).await;
                        }
                        ClientMessage::Login { username, password } => {
                            // Handle user login
                            let result = db::login_user(&username, &password).await;
                            let response = match result {
                                Ok(user) => {
                                    // Update peer map with new user session
                                    let mut peers = peer_map.lock().await;
                                    if let Some(peer) = peers.get_mut(&peer_id) {
                                        peer.user_id = Some(user.id);
                                    }
                                    ServerMessage::LoginSuccess { user_id: user.id }
                                }
                                Err(e) => {
                                    error!("Login error: {:?}", e);
                                    ServerMessage::Error { message: "Login failed".into() }
                                }
                            };
                            let _ = sink.send(serde_json::to_vec(&response).unwrap().into()).await;
                        }
                        ClientMessage::Logout => {
                            // Handle user logout
                            {
                                let mut peers = peer_map.lock().await;
                                if let Some(peer) = peers.get_mut(&peer_id) {
                                    peer.user_id = None;
                                }
                            }
                            let _ = sink.send(serde_json::to_vec(&ServerMessage::LogoutSuccess).unwrap().into()).await;
                        }
                        ClientMessage::CreatePost { channel_id, content } => {
                            // Handle creating a new post
                            let mut peers = peer_map.lock().await;
                            if let Some(peer) = peers.get(&peer_id) {
                                if let Some(user_id) = peer.user_id {
                                    let result = db::create_post(channel_id, user_id, &content).await;
                                    let response = match result {
                                        Ok(post) => ServerMessage::PostCreated { post },
                                        Err(e) => {
                                            error!("Create post error: {:?}", e);
                                            ServerMessage::Error { message: "Post creation failed".into() }
                                        }
                                    };
                                    let _ = sink.send(serde_json::to_vec(&response).unwrap().into()).await;
                                }
                            }
                        }
                        ClientMessage::DeletePost { post_id } => {
                            // Handle deleting a post
                            let mut peers = peer_map.lock().await;
                            if let Some(peer) = peers.get(&peer_id) {
                                if let Some(user_id) = peer.user_id {
                                    let result = db::delete_post(post_id, user_id).await;
                                    let response = match result {
                                        Ok(_) => ServerMessage::PostDeleted { post_id },
                                        Err(e) => {
                                            error!("Delete post error: {:?}", e);
                                            ServerMessage::Error { message: "Post deletion failed".into() }
                                        }
                                    };
                                    let _ = sink.send(serde_json::to_vec(&response).unwrap().into()).await;
                                }
                            }
                        }
                        ClientMessage::UpdateProfile { username, password } => {
                            // Handle updating user profile
                            let mut peers = peer_map.lock().await;
                            if let Some(peer) = peers.get_mut(&peer_id) {
                                if let Some(user_id) = peer.user_id {
                                    let result = db::update_user_profile(user_id, &username, &password).await;
                                    let response = match result {
                                        Ok(_) => ServerMessage::ProfileUpdated,
                                        Err(e) => {
                                            error!("Update profile error: {:?}", e);
                                            ServerMessage::Error { message: "Profile update failed".into() }
                                        }
                                    };
                                    let _ = sink.send(serde_json::to_vec(&response).unwrap().into()).await;
                                }
                            }
                        }
                        ClientMessage::FetchPosts { channel_id } => {
                            // Handle fetching posts for a channel
                            let result = db::fetch_posts(channel_id).await;
                            let response = match result {
                                Ok(posts) => ServerMessage::PostsFetched { posts },
                                Err(e) => {
                                    error!("Fetch posts error: {:?}", e);
                                    ServerMessage::Error { message: "Fetching posts failed".into() }
                                }
                            };
                            let _ = sink.send(serde_json::to_vec(&response).unwrap().into()).await;
                        }
                        ClientMessage::SubscribeToNotifications => {
                            // Handle subscribing to notifications
                            let mut peers = peer_map.lock().await;
                            if let Some(peer) = peers.get(&peer_id) {
                                if let Some(user_id) = peer.user_id {
                                    let result = db::subscribe_to_notifications(user_id).await;
                                    let response = match result {
                                        Ok(_) => ServerMessage::SubscribedToNotifications,
                                        Err(e) => {
                                            error!("Subscribe to notifications error: {:?}", e);
                                            ServerMessage::Error { message: "Subscription failed".into() }
                                        }
                                    };
                                    let _ = sink.send(serde_json::to_vec(&response).unwrap().into()).await;
                                }
                            }
                        }
                        ClientMessage::UnsubscribeFromNotifications => {
                            // Handle unsubscribing from notifications
                            let mut peers = peer_map.lock().await;
                            if let Some(peer) = peers.get(&peer_id) {
                                if let Some(user_id) = peer.user_id {
                                    let result = db::unsubscribe_from_notifications(user_id).await;
                                    let response = match result {
                                        Ok(_) => ServerMessage::UnsubscribedFromNotifications,
                                        Err(e) => {
                                            error!("Unsubscribe from notifications error: {:?}", e);
                                            ServerMessage::Error { message: "Unsubscription failed".into() }
                                        }
                                    };
                                    let _ = sink.send(serde_json::to_vec(&response).unwrap().into()).await;
                                }
                            }
                        }
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
