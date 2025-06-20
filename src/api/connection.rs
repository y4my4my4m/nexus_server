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

use crate::db;
use crate::services::{UserService, ChatService, NotificationService, BroadcastService};
use regex;

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
        
        loop {
            tokio::select! {
                Some(Ok(msg)) = stream.next() => {
                    match bincode::deserialize::<ClientMessage>(&msg) {
                        Ok(message) => {
                            tracing::info!("Parsed ClientMessage: {:?}", message);
                            
                            match message {
                                ClientMessage::Register { username, password } => {
                                    match UserService::register(&username, &password, &peer_map_task).await {
                                        Ok(user) => {
                                            // Update peer map
                                            let mut peers = peer_map_task.lock().await;
                                            if let Some(peer) = peers.get_mut(&peer_id) {
                                                peer.user_id = Some(user.id);
                                            }
                                            current_user = Some(user.clone());
                                            let response = ServerMessage::AuthSuccess(user);
                                            let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                        }
                                        Err(e) => {
                                            let response = ServerMessage::AuthFailure(e.to_string());
                                            let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                        }
                                    }
                                }
                                ClientMessage::Login { username, password } => {
                                    match UserService::login(&username, &password, &peer_map_task).await {
                                        Ok(user) => {
                                            // Update peer map
                                            let mut peers = peer_map_task.lock().await;
                                            if let Some(peer) = peers.get_mut(&peer_id) {
                                                peer.user_id = Some(user.id);
                                            }
                                            current_user = Some(user.clone());
                                            let response = ServerMessage::AuthSuccess(user);
                                            let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                        }
                                        Err(e) => {
                                            let response = ServerMessage::AuthFailure(e.to_string());
                                            let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                        }
                                    }
                                }
                                ClientMessage::Logout => {
                                    if let Some(user) = &current_user {
                                        UserService::logout(user, &peer_map_task).await;
                                    }
                                    
                                    let mut peers = peer_map_task.lock().await;
                                    if let Some(peer) = peers.get_mut(&peer_id) {
                                        peer.user_id = None;
                                    }
                                    current_user = None;
                                }
                                ClientMessage::UpdatePassword(new_password) => {
                                    if let Some(user) = &current_user {
                                        let _ = UserService::update_password(user.id, &new_password).await;
                                    }
                                }
                                ClientMessage::UpdateColor(color) => {
                                    if let Some(user) = &current_user {
                                        let color_str = match color.0 {
                                            ratatui::style::Color::Rgb(r, g, b) => format!("#{:02X}{:02X}{:02X}", r, g, b),
                                            other => format!("{:?}", other),
                                        };
                                        if let Ok(updated_user) = UserService::update_color(user.id, &color_str, &peer_map_task).await {
                                            current_user = Some(updated_user);
                                        }
                                    }
                                }
                                ClientMessage::UpdateProfile { bio, url1, url2, url3, location, profile_pic, cover_banner } => {
                                    if let Some(user) = &current_user {
                                        let _ = UserService::update_profile(
                                            user.id, bio, url1, url2, url3, location, profile_pic, cover_banner, &peer_map_task
                                        ).await;
                                    }
                                }
                                ClientMessage::GetProfile { user_id } => {
                                    match UserService::get_profile(user_id).await {
                                        Ok(profile) => {
                                            let response = ServerMessage::Profile(profile);
                                            let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                        }
                                        Err(e) => {
                                            let response = ServerMessage::Notification(format!("Failed to load profile: {}", e), true);
                                            let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                        }
                                    }
                                }
                                ClientMessage::GetUserList => {
                                    match UserService::get_user_list(&peer_map_task).await {
                                        Ok(users) => {
                                            let response = ServerMessage::UserList(users);
                                            let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                        }
                                        Err(_) => {
                                            let response = ServerMessage::Notification("Failed to get user list".to_string(), true);
                                            let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                        }
                                    }
                                }
                                ClientMessage::SendChannelMessage { channel_id, content } => {
                                    if let Some(user) = &current_user {
                                        let _ = ChatService::send_channel_message(channel_id, user, &content, &peer_map_task).await;
                                    }
                                }
                                ClientMessage::SendDirectMessage { to, content } => {
                                    if let Some(user) = &current_user {
                                        let _ = ChatService::send_direct_message(user, to, &content, &peer_map_task).await;
                                    }
                                }
                                ClientMessage::GetChannelMessages { channel_id, before } => {
                                    match ChatService::get_channel_messages(channel_id, before, 50).await {
                                        Ok((messages, history_complete)) => {
                                            let response = ServerMessage::ChannelMessages { channel_id, messages, history_complete };
                                            let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                        }
                                        Err(_) => {
                                            let response = ServerMessage::Notification("Failed to load messages".to_string(), true);
                                            let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                        }
                                    }
                                }
                                ClientMessage::GetDirectMessages { user_id, before } => {
                                    if let Some(user) = &current_user {
                                        match ChatService::get_direct_messages(user.id, user_id, before, 50).await {
                                            Ok((messages, history_complete)) => {
                                                let response = ServerMessage::DirectMessages { user_id, messages, history_complete };
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
                                            Err(_) => {
                                                let response = ServerMessage::Notification("Failed to load DMs".to_string(), true);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
                                        }
                                    }
                                }
                                ClientMessage::GetChannelUserList { channel_id } => {
                                    match ChatService::get_channel_users(channel_id, &peer_map_task).await {
                                        Ok(users) => {
                                            let response = ServerMessage::ChannelUserList { channel_id, users };
                                            let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                        }
                                        Err(_) => {
                                            let response = ServerMessage::Notification("Failed to get channel users".to_string(), true);
                                            let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                        }
                                    }
                                }
                                ClientMessage::GetDMUserList => {
                                    if let Some(user) = &current_user {
                                        match ChatService::get_dm_user_list(user.id, &peer_map_task).await {
                                            Ok(users) => {
                                                let response = ServerMessage::DMUserList(users);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
                                            Err(_) => {
                                                let response = ServerMessage::Notification("Failed to get DM user list".to_string(), true);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
                                        }
                                    }
                                }
                                ClientMessage::GetNotifications { before } => {
                                    if let Some(user) = &current_user {
                                        match NotificationService::get_notifications(user.id, before).await {
                                            Ok((notifications, history_complete)) => {
                                                let response = ServerMessage::Notifications { notifications, history_complete };
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
                                            Err(_) => {
                                                let response = ServerMessage::Notification("Failed to get notifications".to_string(), true);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
                                        }
                                    }
                                }
                                ClientMessage::MarkNotificationRead { notification_id } => {
                                    let _ = NotificationService::mark_notification_read(notification_id).await;
                                }
                                ClientMessage::GetForums => {
                                    let forums = db::forums::db_get_forums().await.unwrap_or_default();
                                    let response = ServerMessage::Forums(forums);
                                    let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                }
                                ClientMessage::GetServers => {
                                    if let Some(user) = &current_user {
                                        let servers = db::servers::db_get_user_servers(user.id).await.unwrap_or_default();
                                        let response = ServerMessage::Servers(servers);
                                        let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                    }
                                }
                                ClientMessage::CreatePost { thread_id, content } => {
                                    if let Some(user) = &current_user {
                                        match db::forums::db_create_post(thread_id, user.id, &content).await {
                                            Ok(_) => {
                                                let forums = db::forums::db_get_forums().await.unwrap_or_default();
                                                let response = ServerMessage::Forums(forums);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
                                            Err(e) => {
                                                let response = ServerMessage::Notification(format!("Failed to create post: {}", e), true);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
                                        }
                                    }
                                }
                                ClientMessage::CreateThread { forum_id, title, content } => {
                                    if let Some(user) = &current_user {
                                        match db::forums::db_create_thread(forum_id, &title, user.id, &content).await {
                                            Ok(_) => {
                                                let forums = db::forums::db_get_forums().await.unwrap_or_default();
                                                let response = ServerMessage::Forums(forums);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
                                            Err(e) => {
                                                let response = ServerMessage::Notification(format!("Failed to create thread: {}", e), true);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
                                        }
                                    }
                                }
                                ClientMessage::DeletePost(_) | ClientMessage::DeleteThread(_) => {
                                    // TODO: Implement delete functionality
                                    let response = ServerMessage::Notification("Delete functionality not yet implemented".to_string(), true);
                                    let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                }
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
