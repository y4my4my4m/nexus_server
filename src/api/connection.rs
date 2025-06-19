// server/src/api/connection.rs

use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Mutex};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use uuid::Uuid;
use futures::{SinkExt, StreamExt};
use tracing::error;
use common::{ClientMessage, ServerMessage};

use crate::db;

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
    let mut sink = sink; // make sink mutable for both message handling and rx loop
    tokio::spawn(async move {
        loop {
            tokio::select! {
                Some(Ok(msg)) = stream.next() => {
                    match bincode::deserialize::<ClientMessage>(&msg) {
                        Ok(message) => {
                            tracing::info!("Parsed ClientMessage: {:?}", message);
                            match message {
                                ClientMessage::Register { username, password } => {
                                    let result = db::users::db_register_user(&username, &password, "Reset", "User").await;
                                    let response = match result {
                                        Ok(profile) => ServerMessage::AuthSuccess(common::User {
                                            id: profile.id,
                                            username: profile.username,
                                            color: profile.color,
                                            role: profile.role,
                                            profile_pic: profile.profile_pic,
                                            cover_banner: profile.cover_banner,
                                            status: common::UserStatus::Connected,
                                        }),
                                        Err(e) => ServerMessage::AuthFailure(e),
                                    };
                                    let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                }
                                ClientMessage::Login { username, password } => {
                                    let result = db::users::db_login_user(&username, &password).await;
                                    let response = match result {
                                        Ok(profile) => ServerMessage::AuthSuccess(common::User {
                                            id: profile.id,
                                            username: profile.username,
                                            color: profile.color,
                                            role: profile.role,
                                            profile_pic: profile.profile_pic,
                                            cover_banner: profile.cover_banner,
                                            status: common::UserStatus::Connected,
                                        }),
                                        Err(e) => ServerMessage::AuthFailure(e),
                                    };
                                    let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                }
                                ClientMessage::Logout => {
                                    let mut peers = peer_map_task.lock().await;
                                    if let Some(peer) = peers.get_mut(&peer_id) {
                                        peer.user_id = None;
                                    }
                                }
                                ClientMessage::GetForums => {
                                    let forums = db::forums::db_get_forums().await.unwrap_or_default();
                                    let response = ServerMessage::Forums(forums);
                                    let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                }
                                ClientMessage::GetServers => {
                                    let servers = db::servers::db_get_user_servers(peer_map_task.lock().await.get(&peer_id).and_then(|p| p.user_id).unwrap()).await.unwrap_or_default();
                                    let response = ServerMessage::Servers(servers);
                                    let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                }
                                ClientMessage::GetChannelMessages { channel_id, before } => {
                                    let (messages, history_complete) = db::channels::db_get_channel_messages(channel_id, before).await.unwrap_or_default();
                                    let response = ServerMessage::ChannelMessages { channel_id, messages, history_complete };
                                    let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                }
                                ClientMessage::GetChannelUserList { channel_id } => {
                                    let users = db::channels::db_get_channel_user_list(channel_id).await.unwrap_or_default();
                                    let response = ServerMessage::ChannelUserList { channel_id, users };
                                    let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                }
                                ClientMessage::GetDMUserList => {
                                    if let Some(user_id) = peer_map_task.lock().await.get(&peer_id).and_then(|p| p.user_id) {
                                        let users = db::messages::db_get_dm_user_list(user_id).await.unwrap_or_default();
                                        let response = ServerMessage::DMUserList(users);
                                        let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                    }
                                }
                                ClientMessage::GetDirectMessages { user_id, before } => {
                                    if let Some(my_id) = peer_map_task.lock().await.get(&peer_id).and_then(|p| p.user_id) {
                                        let (messages, history_complete) = db::messages::db_get_direct_messages(my_id, user_id, before).await.unwrap_or_default();
                                        let response = ServerMessage::DirectMessages { user_id, messages, history_complete };
                                        let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                    }
                                }
                                ClientMessage::SendDirectMessage { to, content } => {
                                    if let Some(from) = peer_map_task.lock().await.get(&peer_id).and_then(|p| p.user_id) {
                                        let timestamp = chrono::Utc::now().timestamp();
                                        let _ = db::messages::db_store_direct_message(from, to, &content, timestamp).await;
                                    }
                                }
                                ClientMessage::SendChannelMessage { channel_id, content } => {
                                    if let Some(sent_by) = peer_map_task.lock().await.get(&peer_id).and_then(|p| p.user_id) {
                                        let timestamp = chrono::Utc::now().timestamp();
                                        let _ = db::channels::db_create_channel_message(channel_id, sent_by, timestamp, &content).await;
                                    }
                                }
                                ClientMessage::GetNotifications { before } => {
                                    if let Some(user_id) = peer_map_task.lock().await.get(&peer_id).and_then(|p| p.user_id) {
                                        let (notifications, history_complete) = db::notifications::db_get_notifications(user_id, before).await.unwrap_or_default();
                                        let response = ServerMessage::Notifications { notifications, history_complete };
                                        let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                    }
                                }
                                ClientMessage::MarkNotificationRead { notification_id } => {
                                    let _ = db::notifications::db_mark_notification_read(notification_id).await;
                                }
                                ClientMessage::UpdatePassword(new_password) => {
                                    if let Some(user_id) = peer_map_task.lock().await.get(&peer_id).and_then(|p| p.user_id) {
                                        let _ = db::users::db_update_user_password(user_id, &new_password).await;
                                    }
                                }
                                ClientMessage::UpdateProfile { bio, url1, url2, url3, location, profile_pic, cover_banner } => {
                                    if let Some(user_id) = peer_map_task.lock().await.get(&peer_id).and_then(|p| p.user_id) {
                                        let _ = db::users::db_update_user_profile(user_id, bio, url1, url2, url3, location, profile_pic, cover_banner).await;
                                    }
                                }
                                ClientMessage::UpdateColor(color) => {
                                    if let Some(user_id) = peer_map_task.lock().await.get(&peer_id).and_then(|p| p.user_id) {
                                        let color_str = match color.0 {
                                            ratatui::style::Color::Rgb(r, g, b) => format!("#{:02X}{:02X}{:02X}", r, g, b),
                                            other => format!("{:?}", other),
                                        };
                                        let _ = db::users::db_update_user_color(user_id, &color_str).await;
                                    }
                                }
                                ClientMessage::CreatePost { thread_id, content } => {
                                    if let Some(user_id) = peer_map_task.lock().await.get(&peer_id).and_then(|p| p.user_id) {
                                        match db::forums::db_create_post(thread_id, user_id, &content).await {
                                            Ok(_) => {
                                                let response = ServerMessage::Notification("Post created successfully".to_string(), false);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
                                            Err(e) => {
                                                let response = ServerMessage::Notification(format!("Failed to create post: {}", e), true);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
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
        peers.remove(&peer_id);
    });
}
